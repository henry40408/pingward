Fixtures for internal/cron, internal/tz and internal/markdown come from one throwaway Rust program
(capture), run once inside a Docker container. The program is NOT part of the repo.

Pinned versions (same as Cargo.lock): chrono 0.4.45, chrono-tz 0.10.4, cron 0.17.0, serde_json 1.0.154.
markdown cases come from the repo's own src/markdown.rs, included read-only with #[path].

How it was run (rust:1-slim-trixie, --cpus 2, one container at a time, removed afterwards):
  docker run --name pw-capture --cpus 2 -v <repo>:/repo:ro -v <capture dir>:/work rust:1-slim-trixie \
    sh -c 'cd /work && cargo run --release -- /work/out'
  docker rm pw-capture
The program writes four files, which are copied into the packages:
  cron_fixtures.json   -> internal/cron/testdata/fixtures.json
  tz_names.txt         -> internal/tz/testdata/chrono_tz_names.txt
  tz_parse.json        -> internal/tz/testdata/chrono_tz_parse.json
  markdown_cases.json  -> internal/markdown/testdata/cases.json

Contents
- cron fixtures.json: "valid" holds expressions with, per anchor and zone, the next five firing
  instants (UTC, RFC 3339 with nanoseconds). "invalid" holds expressions with the exact cron
  crate error text. Anchors cover DST gaps and overlaps; zones include UTC, Asia/Taipei and
  zones with DST.
- tz chrono_tz_names.txt: chrono_tz::TZ_VARIANTS in crate order; internal/tz/gen.go turns it into
  names.go. chrono_tz_parse.json: what str::parse::<Tz>() returns for probe inputs (null = rejected),
  which pins the case-sensitivity rules.
- markdown cases.json: input and exact HTML output of the Rust renderer, including XSS probes.

To regenerate, repeat the run above and diff the output against the committed files. A diff means
a crate version moved or the Rust source changed: review it, do not just overwrite.
