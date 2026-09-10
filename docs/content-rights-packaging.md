# Local content maintenance and packaging

The September 9, 2026 local candidate for clonk-org/clonk-rs-content#94 keeps
all current imports available for maintenance. Its `packs.toml` distinguishes
`licensed` (a recorded grant), `assumed` (the maintainer's explicit decision
despite incomplete records), and `excluded` (omit the scope). An assumption is
not a new licence or evidence that the original rights holder granted it.
The content repository's `third_party/RIGHTS.md` records the evidence and the
decision; `CONTENT-NOTICES.md` accompanies the data.

Engine installers and the content update archive use the same scope selection
rules. Narrower scopes take precedence; required notices and evidence ship
with selected content. Both packagers refuse omitted definition dependencies,
empty included scopes, unsafe or colliding filenames, and exclusions hidden
inside a packed file that would otherwise still ship.

Older content pins retain their existing packaging behavior. A manifest with
rights decisions must include the distribution policy: deleting its header
cannot silently activate the historical path. The four previously hardcoded
classic packs remain mandatory only for historical pins. Current manifests
make the individual decisions explicit.

The resource example `rights_inventory` recursively reads directory and packed
C4Groups without rewriting assets. Build it with:

```sh
cargo build -p clonk-resources --example rights_inventory
```

In the candidate content checkout:

```sh
uv run --python 3.13 python tools/rights_inventory.py record --scanner /path/to/clonk-rs/target/debug/examples/rights_inventory
uv run --python 3.13 python tools/rights_inventory.py check
```

The inventory binds decisions to physical resource bytes, lists groups and
embedded notices, and records explicit scenario definition references. Static
resolution does not discover arbitrary script loads or prove playability.
Thirteen existing references to `MetalMagic.c4f/Misc.c4d` are absent in the
baseline and recorded under clonk-org/clonk-rs-content#80. No game data was
changed to conceal that gap.

The two repositories build independently, so the small selection module is
copied into both. When changing its rules, update both files and run both
sets of tests. Verify the copies with:

```sh
cmp xtask/src/content_distribution.rs /path/to/clonk-rs-content/tools/pack-content/src/distribution.rs
```

The engine content pin remains unchanged. The content candidate and engine
changes are submitted in separate pull requests for review. The inventory was
prepared locally without contacting rights holders or requesting additional
permission; existing release archives remain unchanged.
