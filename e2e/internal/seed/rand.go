package seed

// mulberry32 is JavaScript's mulberry32, bit for bit. It drives every jitter,
// duration and ping UUID, so any change alters every committed PNG.
type mulberry32 struct{ state uint32 }

func (m *mulberry32) next() float64 {
	m.state += 0x6d2b79f5
	seed := m.state
	t := (seed ^ (seed >> 15)) * (1 | seed)
	t = (t + (t^(t>>7))*(0x3d|t)) ^ t
	return float64(t^(t>>14)) / 4294967296.0
}

// between returns a value in [low, high).
func (m *mulberry32) between(low, high float64) float64 {
	// float64() stops the compiler fusing the multiply and add (arm64 does),
	// which would change the last bit of the Rust-pinned output.
	return low + float64((high-low)*m.next())
}

// index is Math.floor(rand() * length).
func (m *mulberry32) index(length int) int {
	// The modulo only guards the float rounding edge.
	return int(m.next()*float64(length)) % length
}
