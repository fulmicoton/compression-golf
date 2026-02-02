# Fulmicoton Codec Description

This document describes the implementation of the `fulmicoton` codec for the compression-golf challenge.

## Overview

The codec achieves high compression (~5.74 MB for 1M events) by:
1.  **Parsing & Preprocessing**: Converting raw `(EventKey, EventValue)` pairs into a structured `ParsedEvent` format.
2.  **Columnar Storage**: Transposing data into separate columns to maximize pattern matching within Zstd/ANS contexts.
3.  **Monotonic Repair (Timestamps)**: Identifying and encoding the minimal set of "bubble swaps" needed to make the timestamp sequence strictly monotonic, enabling high-efficiency Histogram encoding.
4.  **Specialized Entropy Coding**:
    *   Using **rANS (Asymmetric Numeral Systems)** for byte-aligned indices, large-integer histograms, and u64 sequences.
    *   Using **Elias-Fano** for sorted u64 sequences (repo IDs).
    *   Using **Zstd (level 22)** for structured string data.
5.  **Custom Binary Container**: Concatenating blobs with VInt-prefixed lengths to eliminate serialization overhead.

## Data Structure & Pipeline

### 1. Event IDs (`event_ids`)
*   **Transformation**: `PositiveDelta -> [first as VInt] + [rest as AnsU64]`.
*   **Rationale**: IDs are numeric strings that are strictly increasing when sorted by ID. Delta encoding reduces them to small integers. The first delta is encoded as VInt, while the remaining deltas use AnsU64 for near-optimal entropy coding.

### 2. Event Types (`event_types`)
*   **Transformation**: `u8 index -> AnsBijection`.
*   **Rationale**: There are only 14 unique types. ANS provides near-optimal entropy coding for this small alphabet.

### 3. Timestamps (`created_ats`)
*   **Challenge**: Timestamps are "almost" sorted by ID but contain enough jitter to break simple monotonic compressors.
*   **Transformation**:
    1.  **identify_permutation**: Uses a bubble-sort-inspired algorithm to find the sequence of `(advance, swap)` operations required to sort the timestamps.
    2.  **Move Encoding**: Encodes the advances and swaps as separate VInt columns, then compresses with `Zstd(22)`.
    3.  **Histogram**: The sorted timestamps are converted to a frequency table (counts per second).
    4.  **AnsU64**: The histogram counts are encoded using a specialized ANS implementation that handles `u64` symbols by mapping them to dense ranks.

### 4. Repositories
*   **Dictionary Encoding**: Deduplicates `(id, owner, suffix)` tuples.
*   **Indices (Hybrid Encoding)**:
    *   Top 2047 most frequent repos are mapped to ANS-2048 symbols.
    *   Remaining repos use symbol 2047 ("other") and are stored separately using columnar U24 encoding (ANS for high byte, Zstd for low/mid bytes).
    *   The mapping table is stored as `VInt -> Zstd`.
*   **Metadata**:
    *   **IDs**: `Elias-Fano -> Zstd(22)`. Elias-Fano provides near-optimal encoding for sorted sequences by splitting values into low bits (fixed-width packed) and high bits (unary-coded bitvector).
    *   **Names**: Splits "owner/repo" into two columns. Joins owners and suffixes with newlines, separates them with a null byte, and compresses the entire block with `Zstd(22)`. This allows Zstd to find repetitive owner strings across the entire dictionary.

## Final Serialization
Concatenation of 7 blobs:
1.  `event_ids` (VInt + AnsU64)
2.  `event_type_indices` (ANS)
3.  `dict_event_types` (Zstd)
4.  `created_ats` (Permutation + Histogram + AnsU64)
5.  `repo_indices` (Hybrid: ANS-2048 + columnar U24)
6.  `dict_repo_ids` (Elias-Fano + Zstd)
7.  `dict_repo_names` (Zstd)

Each blob is prefixed with its length as a VInt.

## Performance (vs Naive)
*   **Naive Size**: 210,727,389 bytes
*   **Fulmicoton Size**: ~5,736,674 bytes
*   **Reduction**: ~97.3%