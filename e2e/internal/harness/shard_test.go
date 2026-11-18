package harness

import "testing"

func TestParseShardRejectsMalformedValues(t *testing.T) {
	for _, value := range []string{"3", "0/3", "4/3", "a/b", "1/", "/3"} {
		if _, err := ParseShard(value); err == nil {
			t.Errorf("ParseShard(%q) accepted it", value)
		}
	}
}

func TestParseShardEmptyMeansNoSharding(t *testing.T) {
	shard, err := ParseShard("")
	if shard != nil || err != nil {
		t.Fatalf("ParseShard(\"\") = %v, %v", shard, err)
	}
}

func TestEveryKeyBelongsToExactlyOneShard(t *testing.T) {
	shards := make([]*Shard, 3)
	for i := range shards {
		shard, err := ParseShard([]string{"1/3", "2/3", "3/3"}[i])
		if err != nil {
			t.Fatal(err)
		}
		shards[i] = shard
	}
	for _, key := range []string{"a", "b", "c", "d", "e", "f", "g"} {
		owners := 0
		for _, shard := range shards {
			if shard.Includes(key) {
				owners++
			}
		}
		if owners != 1 {
			t.Errorf("%q belongs to %d shards", key, owners)
		}
	}
}

func TestEnsureSelectedFailsOnAnEmptyShard(t *testing.T) {
	shard, _ := ParseShard("1/2")
	if shard.EnsureSelected() == nil {
		t.Fatal("an empty shard passed")
	}
}
