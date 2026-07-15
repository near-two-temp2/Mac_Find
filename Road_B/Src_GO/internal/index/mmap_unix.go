//go:build darwin || linux

package index

import (
	"fmt"
	"os"

	"golang.org/x/sys/unix"
)

// mmapFile opens path and maps it read-only into memory. The returned slice
// aliases the mapping directly, so it must be released with munmap.
func mmapFile(path string) ([]byte, error) {
	f, err := os.Open(path)
	if err != nil {
		return nil, err
	}
	defer f.Close()

	fi, err := f.Stat()
	if err != nil {
		return nil, err
	}
	size := int(fi.Size())
	if size == 0 {
		return nil, fmt.Errorf("empty index file: %s", path)
	}

	data, err := unix.Mmap(int(f.Fd()), 0, size,
		unix.PROT_READ, unix.MAP_PRIVATE)
	if err != nil {
		return nil, fmt.Errorf("mmap %s: %w", path, err)
	}
	return data, nil
}

// munmap releases a mapping created by mmapFile.
func munmap(data []byte) error {
	if len(data) == 0 {
		return nil
	}
	return unix.Munmap(data)
}
