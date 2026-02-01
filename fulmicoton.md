# Fulmicoton Codec Description

This document describes the implementation of the `fulmicoton` codec for the compression-golf challenge.

## Overview

The codec achieves high compression (~5.93 MB for 1M events) by:
1.  **Parsing & Preprocessing**: Converting raw `(EventKey, EventValue)` pairs into a structured `ParsedEvent` format.
2.  **Columnar Storage**: Transposing data into separate columns to maximize pattern matching within Zstd/ANS contexts.
3.  **Monotonic Repair (Timestamps)**: Identifying and encoding the minimal set of "bubble swaps" needed to make the timestamp sequence strictly monotonic, enabling high-efficiency Histogram encoding.
4.  **Specialized Entropy Coding**: 
    *   Using **rANS (Asymmetric Numeral Systems)** for byte-aligned indices and large-integer histograms.
    *   Using **Zstd (level 22)** for structured string data and sparse deltas.
5.  **Custom Binary Container**: Concatenating blobs with VInt-prefixed lengths to eliminate serialization overhead.

## Data Structure & Pipeline

### 1. Event IDs (`event_ids`)
*   **Transformation**: `PositiveDelta -> VInt -> Zstd(22)`.
*   **Rationale**: IDs are numeric strings that are strictly increasing when sorted by ID. Delta encoding reduces them to small integers.

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
*   **Dictionary Encoding**: deduplicates `(id, owner, suffix)` tuples.
*   **Indices**: `U24 (3-byte) -> Zstd(22)`.
*   **Metadata**: 
    *   **IDs**: `U64Delta -> VInt -> Zstd(22)`.
    *   **Names**: Splits "owner/repo" into two columns. Joins owners and suffixes with newlines, separates them with a null byte, and compresses the entire block with `Zstd(22)`. This allows Zstd to find repetitive owner strings across the entire dictionary.

## Final Serialization
Concatenation of 7 blobs:
1.  `event_ids` (Zstd)
2.  `event_type_indices` (ANS)
3.  `dict_event_types` (Zstd)
4.  `created_ats` (Zstd/ANS - Permutation + Histogram)
5.  `repo_indices` (Zstd)
6.  `dict_repo_ids` (Zstd)
7.  `dict_repo_names` (Zstd)

Each blob is prefixed with its length as a VInt.

## Performance (vs Naive)
*   **Naive Size**: 210,727,389 bytes
*   **Fulmicoton Size**: ~5,930,460 bytes
*   **Reduction**: ~97.2%