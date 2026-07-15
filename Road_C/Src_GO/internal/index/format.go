// Package index implements a self-built binary filename index, modeled on the
// Cling parallel-array / mmap-friendly design (open-source-analysis.md §3.3).
//
// On-disk layout (little-endian):
//
//	Header (32 bytes):
//	  magic       uint64   "MCFIDX01"
//	  version     uint32   = 1
//	  entryCount  uint32
//	  bytesLen    uint64   length of the packed path-bytes blob
//	  reserved    uint64
//	Parallel arrays (entryCount elements each):
//	  masks       []uint64  path character bitmask
//	  bnMasks     []uint64  basename character bitmask
//	  offsets     []uint32  offset into the bytes blob
//	  lengths     []uint32  path byte length
//	  bnStarts    []uint32  basename start offset within the path
//	  isDirs      []uint8   1 = directory, 0 = file
//	Blob:
//	  bytes       []byte    concatenated lowercased UTF-8 paths
//
// The reader keeps the whole file in a single []byte and exposes each path as a
// sub-slice, so lookups avoid per-entry allocation.
package index

import "encoding/binary"

const (
	formatVer   uint32 = 1
	headerSize         = 32
	entryStride        = 8 + 8 + 4 + 4 + 4 + 1 // masks+bnMasks+offset+length+bnStart+isDir
)

// magicBytes marks a valid index file. A corrupt/truncated file fails the load
// and triggers the searchfs() fallback.
var magicBytes = []byte("MCFIDX01")

func putHeader(buf []byte, entryCount uint32, bytesLen uint64) {
	copy(buf[0:8], magicBytes)
	binary.LittleEndian.PutUint32(buf[8:12], formatVer)
	binary.LittleEndian.PutUint32(buf[12:16], entryCount)
	binary.LittleEndian.PutUint64(buf[16:24], bytesLen)
	binary.LittleEndian.PutUint64(buf[24:32], 0)
}
