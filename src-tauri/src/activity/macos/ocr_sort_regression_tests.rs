//! Reading order regressions for `sort_and_join_observations`: the same
//! observations read the same whatever order Vision returns them in, and a
//! line is anchored at its first observation instead of chaining through
//! near neighbours.

use super::*;

const PERMUTATIONS: [[usize; 3]; 6] = [
    [0, 1, 2],
    [0, 2, 1],
    [1, 0, 2],
    [1, 2, 0],
    [2, 0, 1],
    [2, 1, 0],
];

fn observation(text: &str, top_y: f64, left_x: f64, height: f64) -> TextObservation {
    TextObservation {
        text: text.to_string(),
        top_y,
        left_x,
        height,
    }
}

fn assert_same_reading_order(observations: &[TextObservation; 3], expected: &str) {
    for order in PERMUTATIONS {
        let input = Vec::from(order.map(|index| observations[index].clone()));
        assert_eq!(
            sort_and_join_observations(input),
            expected,
            "input order {order:?}"
        );
    }
}

/// The middle observation is within tolerance of both neighbours, which are
/// not within tolerance of each other: the top one anchors the line, so the
/// bottom one starts the next line.
#[test]
fn a_tolerance_chain_reads_the_same_for_every_input_order() {
    assert_same_reading_order(
        &[
            observation("top right", 0.90, 0.3, 0.1),
            observation("middle", 0.86, 0.2, 0.1),
            observation("bottom left", 0.82, 0.1, 0.1),
        ],
        "middle\ntop right\nbottom left",
    );
}

/// Tolerance is half the taller of the anchor and the candidate. A taller
/// observation an eighth below the anchor joins its line; one exactly half its
/// height below starts a new line, even though it is within tolerance of the
/// observation that joined.
#[test]
fn a_line_ends_at_half_the_taller_height_below_its_anchor() {
    assert_same_reading_order(
        &[
            observation("anchor", 0.75, 0.5, 0.25),
            observation("taller", 0.625, 0.25, 0.5),
            observation("edge", 0.5, 0.0, 0.5),
        ],
        "taller\nanchor\nedge",
    );
}

/// Many observations, each within tolerance of its neighbours but not of the
/// next one down: lines stay anchored at every other observation whether
/// Vision returns them top down or bottom up.
#[test]
fn a_long_tolerance_chain_reads_as_anchored_pairs_in_either_input_order() {
    let chain: Vec<TextObservation> = (0..24)
        .map(|index| {
            let step = f64::from(index) * 0.04;
            let text = format!("observation {index}");
            observation(&text, 0.98 - step, 0.96 - step, 0.1)
        })
        .collect();
    let expected = (0..12)
        .flat_map(|line| [2 * line + 1, 2 * line])
        .map(|index| format!("observation {index}"))
        .collect::<Vec<_>>()
        .join("\n");

    let top_down = chain.clone();
    let bottom_up = chain.into_iter().rev().collect();
    assert_eq!(sort_and_join_observations(top_down), expected);
    assert_eq!(sort_and_join_observations(bottom_up), expected);
}

/// Two observations share a position but not a height. The taller one anchors
/// the line whichever arrives first, so the lower observation within half its
/// height joins that line instead of depending on the input order.
#[test]
fn a_coordinate_tie_anchors_the_line_at_the_taller_observation() {
    assert_same_reading_order(
        &[
            observation("short", 0.875, 0.5, 0.125),
            observation("tall", 0.875, 0.5, 0.5),
            observation("lower left", 0.75, 0.25, 0.125),
        ],
        "lower left\ntall\nshort",
    );
}

/// Observations with the same box read in text order whichever arrives first.
#[test]
fn identical_boxes_read_in_text_order() {
    assert_same_reading_order(
        &[
            observation("beta", 0.5, 0.5, 0.25),
            observation("alpha", 0.5, 0.5, 0.25),
            observation("above", 0.875, 0.25, 0.125),
        ],
        "above\nalpha\nbeta",
    );
}
