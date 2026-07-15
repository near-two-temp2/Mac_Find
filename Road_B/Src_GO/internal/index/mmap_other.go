//go:build !darwin && !linux

package index

import "os"

// mmapFile falls back to a plain read on platforms without a unix mmap. The
// target platform is macOS; this keeps `go build` green everywhere else so
// tooling and IDEs stay happy.
func mmapFile(path string) ([]byte, error) {
	return os.ReadFile(path)
}

// munmap is a no-op for the read-into-memory fallback.
func munmap(data []byte) error { return nil }
