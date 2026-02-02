# Fulmicoton Codec Description

This document describes the implementation of the `fulmicoton` codec for the compression-golf challenge.

## Overview

The codec achieves high compression (~5.71 MB for 1M events) by:
1.  **Parsing & Preprocessing**: Converting raw `(EventKey, EventValue)` pairs into a structured `ParsedEvent` format.
2.  **Columnar Storage**: Transposing data into separate columns to maximize pattern matching within Zstd/ANS contexts.
3.  **Monotonic Repair (Timestamps)**: Identifying and encoding the minimal set of "bubble swaps" needed to make the timestamp sequence strictly monotonic, enabling high-efficiency Histogram encoding.
4.  **Specialized Entropy Coding**:
    *   Using **rANS (Asymmetric Numeral Systems)** for byte-aligned indices and large-integer histograms.
    *   Using **Adaptive Arithmetic Coding** (Order-0 model with range coding) for event ID deltas.
    *   Using **BIC (Binary Interpolative Coding)** for sorted u64 sequences (repo IDs).
    *   Using **Zstd (level 22)** for structured string data.
5.  **Custom Binary Container**: Concatenating blobs with VInt-prefixed lengths to eliminate serialization overhead.

## Data Structure & Pipeline

### 1. Event IDs (`event_ids`)
*   **Transformation**: `PositiveDelta -> AdaptiveMix`.
*   **Rationale**: IDs are numeric strings that are strictly increasing when sorted by ID. Delta encoding reduces them to small integers. The deltas are then encoded using adaptive arithmetic coding (Order-0 model with range coding) for near-optimal entropy coding.

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
    *   **IDs**: `BIC (Binary Interpolative Coding)`. BIC recursively encodes the middle element of a sorted sequence using only the bits needed given the range constraints, achieving near-optimal compression for sorted sequences. Duplicate IDs (from repo renames) are stored separately as delta-encoded VInts.
    *   **Names**: Splits "owner/repo" into two columns. Joins owners and suffixes with newlines, separates them with a null byte, and compresses the entire block with `Zstd(22)`. This allows Zstd to find repetitive owner strings across the entire dictionary.

## Final Serialization
Concatenation of 7 blobs:
1.  `event_ids` (AdaptiveMix)
2.  `event_type_indices` (ANS)
3.  `dict_event_types` (Zstd)
4.  `created_ats` (Permutation + Histogram + AnsU64)
5.  `repo_indices` (Hybrid: ANS-2048 + columnar U24)
6.  `dict_repo_ids` (BIC + VInt for duplicates)
7.  `dict_repo_names` (Zstd)

Each blob is prefixed with its length as a VInt.

## Performance (vs Naive)
*   **Naive Size**: 210,727,389 bytes
*   **Fulmicoton Size**: 5,710,421 bytes
*   **Reduction**: ~97.3%

## Component Sizes
| Component | Size (bytes) |
|-----------|-------------|
| event_ids | 339,186 |
| event_type_indices | 220,318 |
| dict_event_types | 119 |
| created_ats | 24,835 |
| repo_indices | 2,035,949 |
| dict_repo_ids | 405,419 |
| dict_repo_names | 2,684,575 |