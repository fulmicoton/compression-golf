pub fn identify_permutation(mut values: Vec<u64>) -> (Vec<u64>, Vec<(usize, usize)>) {
    let mut moves = Vec::new();
    let mut i = 1;
    let mut advance_count = 0;

    // We effectively perform an insertion sort
    while i < values.len() {
        if values[i] >= values[i-1] {
            advance_count += 1;
            i += 1;
        } else {
            // Need to bubble backwards
            let mut bubble_count = 0;
            let mut j = i;
            while j > 0 && values[j] < values[j-1] {
                values.swap(j, j-1);
                j -= 1;
                bubble_count += 1;
            }

            // We record the advance count *before* this element was processed?
            // "number of elements we advanced without making any transposition"
            // We advanced past indices that were correct.
            // When we hit `i` and it's wrong, we stop.
            // So `advance_count` is the run of good elements.
            moves.push((advance_count, bubble_count));
            advance_count = 0; // Reset

            // After bubbling, the element that was at `i` is now at `j`.
            // The elements from `j` to `i` have shifted.
            // But for the purpose of the scan, we consider `values[0..=i]` to be sorted now.
            // So we can proceed to `i+1`.
            i += 1;
        }
    }

    // If we finished with some advances
    if advance_count > 0 {
        moves.push((advance_count, 0));
    }

    println!("Moves: {}", moves.len());

    (values, moves)
}

pub fn restore_permutation(mut values: Vec<u64>, moves: Vec<(usize, usize)>) -> Vec<u64> {
    let mut actions = Vec::new();
    let mut i = 1;
    for (adv, bubble) in moves {
        i += adv;
        if bubble > 0 {
            actions.push((i, bubble));
            i += 1;
        }
    }

    for (idx, bubble) in actions.into_iter().rev() {
        let src = idx - bubble;
        let dest = idx;
        values[src..=dest].rotate_left(1);
    }
    values
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_identify_permutation_sorted() {
        let input = vec![1, 2, 3, 4, 5];
        let (sorted, moves) = identify_permutation(input.clone());
        assert_eq!(sorted, vec![1, 2, 3, 4, 5]);
        assert_eq!(moves, vec![(4, 0)]);

        let restored = restore_permutation(sorted, moves);
        assert_eq!(restored, input);
    }

    #[test]
    fn test_identify_permutation_reverse() {
        let input = vec![5, 4, 3, 2, 1];
        let (sorted, moves) = identify_permutation(input.clone());
        assert_eq!(sorted, vec![1, 2, 3, 4, 5]);
        assert_eq!(moves, vec![(0, 1), (0, 2), (0, 3), (0, 4)]);

        let restored = restore_permutation(sorted, moves);
        assert_eq!(restored, input);
    }

    #[test]
    fn test_identify_permutation_mixed() {
        // [1, 3, 5, 2, 6]
        let input = vec![1, 3, 5, 2, 6];
        let (sorted, moves) = identify_permutation(input.clone());
        assert_eq!(sorted, vec![1, 2, 3, 5, 6]);
        assert_eq!(moves, vec![(2, 2), (1, 0)]);

        let restored = restore_permutation(sorted, moves);
        assert_eq!(restored, input);
    }
}
