---
name: bundle-fulmicoton
description: Bundle the fulmicoton/ module into a single fulmicoton.rs file, remove tests and dead code, verify it compiles and outputs the same result, then commit to the fulmicoton branch.
---

# Bundle Fulmicoton

Bundle the `src/fulmicoton/` module directory into a single `src/fulmicoton.rs` file for contest submission.

## Steps

### 1. Create the Bundled File

Combine all files from `src/fulmicoton/` into a single `src/fulmicoton.rs` file.

**Source files:**
- `algo.rs` - permutation algorithms
- `bijection.rs` - bijection traits and implementations
- `ans.rs` - ANS entropy encoding
- `columnar.rs` - columnar data structures and parsing
- `mod.rs` - main codec implementation
- `timestamp.rs` - NOT USED, skip entirely

**Transformation rules:**
- Remove all `mod` declarations from mod.rs
- Remove `use super::*` and `use crate::fulmicoton::*` imports
- Inline all referenced code from submodules
- Keep external crate imports: `zstd`, `chrono`, `serde`, `bytes`
- Keep crate imports: `crate::codec::EventCodec`, `crate::{EventKey, EventValue, Repo}`
- Remove all `#[cfg(test)]` blocks and `mod tests` sections
- Remove all `println!` statements
- Remove all unused structs, functions, and impls

### 2. Verify

```bash
cargo build --release --bin compression_golf
cargo run --release --bin compression_golf -- --codec fulmicoton
```

Should show "All verifications passed".

### 3. Commit to fulmicoton Branch

```bash
git checkout fulmicoton
git add src/fulmicoton.rs
git commit -m "Bundle fulmicoton module into single file"
```

## Dead Code to Remove

**From bijection.rs:**
- `LzmaBijection`
- `U24Bijection` (encode_u24_columnar is used instead)
- `DeltaBijection`
- `ZigZagBijection`
- `MonotonicRepairBijection`

**From ans.rs:**
- `AnsU16Bijection`

**Entire files:**
- `timestamp.rs` (TimestampCodec not used)

## Code to Keep

**From algo.rs:**
- `identify_permutation`
- `restore_permutation`

**From bijection.rs:**
- `Bijection` trait
- `ZStdBijection`
- `VIntBijection`
- `PositiveDeltaBijection`
- `U64DeltaBijection`
- `HistogramBijection`
- `MonotonicPermutationBijection`
- `write_vint_local`, `read_vint_local`

**From ans.rs:**
- `AnsBijection`
- `AnsU64Bijection`
- `Ans2048Bijection`
- `write_vint_local`, `read_vint_local`
- Constants: `SCALE_BITS`, `SCALE`, `STATE_LOWER_BOUND`, `SCALE_BITS_2048`, `SCALE_2048`, `STATE_LOWER_BOUND_2048`

**From columnar.rs:**
- `ParsedEvent`
- `ParseBijection`
- `ColumnarEvents`
- `EventsToColumns`

**From mod.rs:**
- `FulmicotonCodec` struct and impls
- `encode_repo_indices_hybrid`, `decode_repo_indices_hybrid`
- `encode_u24_columnar`, `decode_u24_columnar`
- `write_vint`, `read_vint`
- `OTHER_SYMBOL` constant
