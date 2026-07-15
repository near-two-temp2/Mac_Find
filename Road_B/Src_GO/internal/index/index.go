package index

import (
	"encoding/binary"
	"fmt"
	"os"
	"strings"
)

// Magic identifies our on-disk index format. ("MACFINDB" = MAC FIND road-B)
const Magic uint64 = 0x4D414346494E4442 // "MACFINDB" big-endian

// headerSize is the fixed-size header at the top of every .idx file.
//
//	magic       uint64
//	entryCount  uint64
//	bytesLen    uint64
const headerSize = 24

// Index is an in-memory, mmap-backed view of the binary index. All the
// per-entry columns are parallel arrays: entry i is described by masks[i],
// extIDs[i], byteOffsets[i], byteLengths[i] and bnStarts[i]. The raw
// lowercase path bytes live packed together in allBytes.
type Index struct {
	Count int

	Masks       []uint64 // full-path presence bitmask (bits per mask.go)
	BNMasks     []uint64 // basename-only presence bitmask
	ExtIDs      []uint32 // interned lowercase extension id (0 = none)
	ByteOffsets []uint32 // start of the path in AllBytes
	ByteLengths []uint16 // length of the path in AllBytes
	BNStarts    []uint16 // offset of basename within the path
	IsDirs      []uint8  // 1 if the entry is a directory

	AllBytes []byte // packed, lowercase UTF-8 path bytes

	// mmap holds the backing mapping when the index was loaded via Open. It
	// is nil for indexes built in memory. Close unmaps it.
	mmap []byte

	// extNames maps an extension id back to its lowercase string. Index 0 is
	// always the empty extension.
	extNames []string
}

// Path returns the packed lowercase path for entry i.
func (ix *Index) Path(i int) []byte {
	off := ix.ByteOffsets[i]
	return ix.AllBytes[off : off+uint32(ix.ByteLengths[i])]
}

// Basename returns the packed lowercase basename for entry i.
func (ix *Index) Basename(i int) []byte {
	off := ix.ByteOffsets[i] + uint32(ix.BNStarts[i])
	end := ix.ByteOffsets[i] + uint32(ix.ByteLengths[i])
	return ix.AllBytes[off:end]
}

// ExtName returns the lowercase extension string for extension id.
func (ix *Index) ExtName(id uint32) string {
	if int(id) < len(ix.extNames) {
		return ix.extNames[id]
	}
	return ""
}

// ExtCount returns the number of interned extensions (including id 0, the
// empty extension). Used by the search layer to resolve an extension filter.
func (ix *Index) ExtCount() int { return len(ix.extNames) }

// Builder accumulates entries and interns extensions while a scan runs. It is
// not safe for concurrent use; the walker feeds it from a single goroutine.
type Builder struct {
	masks       []uint64
	bnMasks     []uint64
	extIDs      []uint32
	byteOffsets []uint32
	byteLengths []uint16
	bnStarts    []uint16
	isDirs      []uint8
	allBytes    []byte

	extIDByName map[string]uint32
	extNames    []string
}

// NewBuilder returns an empty Builder ready to accept entries.
func NewBuilder() *Builder {
	b := &Builder{
		extIDByName: map[string]uint32{"": 0},
		extNames:    []string{""}, // id 0 == no extension
	}
	return b
}

// Add records one filesystem path. The path is lowercased for storage; the
// original casing is not kept (the fzf matcher is case-insensitive, and
// Road_B mirrors Cling which indexes lowercase bytes only).
func (b *Builder) Add(path string, isDir bool) {
	lower := strings.ToLower(path)
	lb := []byte(lower)

	if len(lb) > 0xFFFF {
		return // paths longer than a uint16 length field are skipped
	}

	off := uint32(len(b.allBytes))
	b.allBytes = append(b.allBytes, lb...)

	// Basename start = byte after the final '/'.
	bnStart := 0
	if idx := strings.LastIndexByte(lower, '/'); idx >= 0 {
		bnStart = idx + 1
	}
	basename := lb[bnStart:]

	b.masks = append(b.masks, MaskFor(lb))
	b.bnMasks = append(b.bnMasks, MaskFor(basename))
	b.extIDs = append(b.extIDs, b.internExt(string(basename)))
	b.byteOffsets = append(b.byteOffsets, off)
	b.byteLengths = append(b.byteLengths, uint16(len(lb)))
	b.bnStarts = append(b.bnStarts, uint16(bnStart))
	if isDir {
		b.isDirs = append(b.isDirs, 1)
	} else {
		b.isDirs = append(b.isDirs, 0)
	}
}

// internExt returns the id for the extension of basename, interning it on
// first sight. A missing or leading-dot-only extension maps to id 0.
func (b *Builder) internExt(basename string) uint32 {
	dot := strings.LastIndexByte(basename, '.')
	if dot <= 0 || dot == len(basename)-1 {
		return 0
	}
	ext := basename[dot+1:]
	if id, ok := b.extIDByName[ext]; ok {
		return id
	}
	id := uint32(len(b.extNames))
	b.extIDByName[ext] = id
	b.extNames = append(b.extNames, ext)
	return id
}

// Len reports how many entries have been added.
func (b *Builder) Len() int { return len(b.masks) }

// Build finalizes the accumulated columns into an in-memory Index.
func (b *Builder) Build() *Index {
	return &Index{
		Count:       len(b.masks),
		Masks:       b.masks,
		BNMasks:     b.bnMasks,
		ExtIDs:      b.extIDs,
		ByteOffsets: b.byteOffsets,
		ByteLengths: b.byteLengths,
		BNStarts:    b.bnStarts,
		IsDirs:      b.isDirs,
		AllBytes:    b.allBytes,
		extNames:    b.extNames,
	}
}

// Save serializes an Index to path in the mmap-friendly binary layout:
// header, then each parallel column contiguously, then the extension table,
// then the packed bytes. Columns are laid out little-endian so that Open can
// mmap the file and reinterpret regions directly on little-endian hosts.
func (ix *Index) Save(path string) error {
	f, err := os.Create(path)
	if err != nil {
		return err
	}
	defer f.Close()

	n := ix.Count
	buf := make([]byte, headerSize)
	binary.LittleEndian.PutUint64(buf[0:], Magic)
	binary.LittleEndian.PutUint64(buf[8:], uint64(n))
	binary.LittleEndian.PutUint64(buf[16:], uint64(len(ix.AllBytes)))
	if _, err := f.Write(buf); err != nil {
		return err
	}

	w := &colWriter{f: f}
	for i := 0; i < n; i++ {
		w.u64(ix.Masks[i])
	}
	for i := 0; i < n; i++ {
		w.u64(ix.BNMasks[i])
	}
	for i := 0; i < n; i++ {
		w.u32(ix.ExtIDs[i])
	}
	for i := 0; i < n; i++ {
		w.u32(ix.ByteOffsets[i])
	}
	for i := 0; i < n; i++ {
		w.u16(ix.ByteLengths[i])
	}
	for i := 0; i < n; i++ {
		w.u16(ix.BNStarts[i])
	}
	for i := 0; i < n; i++ {
		w.u8(ix.IsDirs[i])
	}
	if w.err != nil {
		return w.err
	}

	// Extension table: count, then length-prefixed names.
	extCount := len(ix.extNames)
	w.u32(uint32(extCount))
	for _, name := range ix.extNames {
		w.u16(uint16(len(name)))
		if _, err := f.WriteString(name); err != nil {
			return err
		}
	}
	if w.err != nil {
		return w.err
	}

	if _, err := f.Write(ix.AllBytes); err != nil {
		return err
	}
	return nil
}

// colWriter buffers small fixed-width writes and records the first error.
type colWriter struct {
	f   *os.File
	err error
	tmp [8]byte
}

func (w *colWriter) u64(v uint64) {
	if w.err != nil {
		return
	}
	binary.LittleEndian.PutUint64(w.tmp[:8], v)
	_, w.err = w.f.Write(w.tmp[:8])
}

func (w *colWriter) u32(v uint32) {
	if w.err != nil {
		return
	}
	binary.LittleEndian.PutUint32(w.tmp[:4], v)
	_, w.err = w.f.Write(w.tmp[:4])
}

func (w *colWriter) u16(v uint16) {
	if w.err != nil {
		return
	}
	binary.LittleEndian.PutUint16(w.tmp[:2], v)
	_, w.err = w.f.Write(w.tmp[:2])
}

func (w *colWriter) u8(v uint8) {
	if w.err != nil {
		return
	}
	w.tmp[0] = v
	_, w.err = w.f.Write(w.tmp[:1])
}

// Open mmaps an index file and returns a read-only view over it. Callers must
// invoke Close to release the mapping. On failure the mapping is torn down
// before returning the error.
func Open(path string) (*Index, error) {
	data, err := mmapFile(path)
	if err != nil {
		return nil, err
	}
	ix, err := parse(data)
	if err != nil {
		_ = munmap(data)
		return nil, err
	}
	ix.mmap = data
	return ix, nil
}

// Close releases the mmap backing an index opened with Open. It is a no-op
// for in-memory indexes.
func (ix *Index) Close() error {
	if ix.mmap == nil {
		return nil
	}
	err := munmap(ix.mmap)
	ix.mmap = nil
	return err
}

// parse validates the header and slices the mmapped bytes into the parallel
// column arrays. The integer columns are decoded into freshly allocated Go
// slices (portable and endian-safe); only AllBytes aliases the mapping
// directly, which is where the bulk of the file lives.
func parse(data []byte) (*Index, error) {
	if len(data) < headerSize {
		return nil, fmt.Errorf("index too small: %d bytes", len(data))
	}
	if binary.LittleEndian.Uint64(data[0:]) != Magic {
		return nil, fmt.Errorf("bad magic (not a macfind index)")
	}
	n := int(binary.LittleEndian.Uint64(data[8:]))
	bytesLen := int(binary.LittleEndian.Uint64(data[16:]))

	ix := &Index{
		Count:       n,
		Masks:       make([]uint64, n),
		BNMasks:     make([]uint64, n),
		ExtIDs:      make([]uint32, n),
		ByteOffsets: make([]uint32, n),
		ByteLengths: make([]uint16, n),
		BNStarts:    make([]uint16, n),
		IsDirs:      make([]uint8, n),
	}

	r := &colReader{data: data, pos: headerSize}
	for i := 0; i < n; i++ {
		ix.Masks[i] = r.u64()
	}
	for i := 0; i < n; i++ {
		ix.BNMasks[i] = r.u64()
	}
	for i := 0; i < n; i++ {
		ix.ExtIDs[i] = r.u32()
	}
	for i := 0; i < n; i++ {
		ix.ByteOffsets[i] = r.u32()
	}
	for i := 0; i < n; i++ {
		ix.ByteLengths[i] = r.u16()
	}
	for i := 0; i < n; i++ {
		ix.BNStarts[i] = r.u16()
	}
	for i := 0; i < n; i++ {
		ix.IsDirs[i] = r.u8()
	}

	extCount := int(r.u32())
	ix.extNames = make([]string, extCount)
	for i := 0; i < extCount; i++ {
		l := int(r.u16())
		ix.extNames[i] = string(r.bytes(l))
	}
	if r.err != nil {
		return nil, r.err
	}

	if r.pos+bytesLen > len(data) {
		return nil, fmt.Errorf("truncated index: want %d path bytes, have %d",
			bytesLen, len(data)-r.pos)
	}
	ix.AllBytes = data[r.pos : r.pos+bytesLen]
	return ix, nil
}

// colReader walks the file sequentially, mirroring colWriter, and records the
// first out-of-bounds read.
type colReader struct {
	data []byte
	pos  int
	err  error
}

func (r *colReader) need(nbytes int) bool {
	if r.err != nil {
		return false
	}
	if r.pos+nbytes > len(r.data) {
		r.err = fmt.Errorf("index truncated at offset %d", r.pos)
		return false
	}
	return true
}

func (r *colReader) u64() uint64 {
	if !r.need(8) {
		return 0
	}
	v := binary.LittleEndian.Uint64(r.data[r.pos:])
	r.pos += 8
	return v
}

func (r *colReader) u32() uint32 {
	if !r.need(4) {
		return 0
	}
	v := binary.LittleEndian.Uint32(r.data[r.pos:])
	r.pos += 4
	return v
}

func (r *colReader) u16() uint16 {
	if !r.need(2) {
		return 0
	}
	v := binary.LittleEndian.Uint16(r.data[r.pos:])
	r.pos += 2
	return v
}

func (r *colReader) u8() uint8 {
	if !r.need(1) {
		return 0
	}
	v := r.data[r.pos]
	r.pos++
	return v
}

func (r *colReader) bytes(n int) []byte {
	if !r.need(n) {
		return nil
	}
	v := r.data[r.pos : r.pos+n]
	r.pos += n
	return v
}
