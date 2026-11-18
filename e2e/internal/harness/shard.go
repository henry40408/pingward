package harness

import (
	"fmt"
	"hash/fnv"
	"strconv"
	"strings"
	"sync/atomic"
)

// Shard splits the suite across CI jobs: E2E_SHARD=<index>/<total>, 1-based,
// keeps only the scenarios that hash into that shard.
type Shard struct {
	index, total uint64
	selected     atomic.Int64
}

// ParseShard reads an E2E_SHARD value. Empty means no sharding (nil). A
// malformed value is an error rather than a silent full run, which would hide
// a broken CI matrix.
func ParseShard(value string) (*Shard, error) {
	if value == "" {
		return nil, nil
	}
	indexText, totalText, found := strings.Cut(value, "/")
	index, indexErr := strconv.ParseUint(indexText, 10, 64)
	total, totalErr := strconv.ParseUint(totalText, 10, 64)
	if !found || indexErr != nil || totalErr != nil || index < 1 || index > total {
		return nil, fmt.Errorf("E2E_SHARD must look like `2/3`, got `%s`", value)
	}
	return &Shard{index: index, total: total}, nil
}

// Includes reports whether the scenario belongs to this shard. The key must
// tell apart the rows an outline expands into.
func (s *Shard) Includes(key string) bool {
	hash := fnv.New64a()
	_, _ = hash.Write([]byte(key))
	selected := hash.Sum64()%s.total+1 == s.index
	if selected {
		s.selected.Add(1)
	}
	return selected
}

// EnsureSelected fails when the shard selected nothing: the split is wrong,
// and a green run would silently skip those scenarios.
func (s *Shard) EnsureSelected() error {
	selected := s.selected.Load()
	fmt.Printf("shard %d/%d: %d scenarios\n", s.index, s.total, selected)
	if selected == 0 {
		return fmt.Errorf("shard %d/%d selected no scenarios", s.index, s.total)
	}
	return nil
}
