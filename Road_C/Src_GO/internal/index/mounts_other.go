//go:build !darwin

package index

// listMounts is a no-op off macOS: there is no getmntinfo(3) mount table to
// consult, so the build relies solely on the static exclusion list. The product
// only ships on macOS; this stub exists so the package still builds/vets on
// other platforms (e.g. CI linters).
func listMounts() []mount { return nil }

type mount struct {
	point   string
	fstype  string
	isLocal bool
}
