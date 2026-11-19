# status-codes

A development-only crate. It is `publish = false` and no published crate depends on it.

The bin fetches the authoritative IANA HTTP status-code CSV and writes the single snapshot at
`roundhouse-schema/data/status-codes.json`.

`roundhouse-schema::status_codes` embeds that JSON. The status proc macro reads it and emits the
internal `macro_rules!` invocations that `roundhouse::status` expands into wrapper structs. The
build-time analyzer reads the same catalog. Neither path talks to the network.

## Refresh the snapshot

```sh
cargo run -p roundhouse-status-codes --bin update-status-codes
```

Review the diff. The generated identifiers are part of the public API, so a rename needs attention
even when IANA's upstream change is correct.

## Check without writing

```sh
cargo run -p roundhouse-status-codes --bin update-status-codes -- --check
```

This is suitable for a scheduled CI job. Normal builds and tests never fetch the registry.

## Offline conversion

```sh
cargo run -p roundhouse-status-codes --bin update-status-codes -- \
  --input path/to/http-status-codes-1.csv \
  --output /tmp/status-codes.json
```

## Policy

- Keep assigned `2xx`, `4xx` and `5xx` codes as JSON-body wrappers.
- Keep `204`, `205` and `304` as status-only wrappers.
- Skip informational responses, redirects other than `304`, ranges, unassigned rows and
  unused/obsoleted/temporary registrations.
- Override `200` to `Success` (so it cannot shadow `Result::Ok`), `413` to `PayloadTooLarge` and
  `422` to `UnprocessableEntity`, matching the `http` crate's names.
- The snapshot never contains a fetch timestamp, so regeneration produces stable diffs.
