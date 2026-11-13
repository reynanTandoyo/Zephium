//! The cost line under an answer, as decimal strings the panel shows as is.

/// US dollars from micro-dollars, to two significant digits (two decimals
/// from one dollar up).
pub fn usd(micros: u64) -> String {
    if micros == 0 {
        return "0".to_owned();
    }
    let micros = u128::from(micros);
    if micros >= 1_000_000 {
        return decimal((micros + 5_000) / 10_000, 2);
    }
    let digits = micros.ilog10() + 1;
    let places = (6 - digits + 2).min(6);
    let divisor = 10u128.pow(6 - places);
    decimal((micros + divisor / 2) / divisor, places)
}

/// `value / 10^places`, trailing zeros trimmed.
fn decimal(value: u128, places: u32) -> String {
    let scale = 10u128.pow(places);
    let whole = value / scale;
    let fraction = value % scale;
    if fraction == 0 {
        return whole.to_string();
    }
    let fraction = format!("{fraction:0width$}", width = places as usize);
    format!("{whole}.{}", fraction.trim_end_matches('0'))
}

/// A price per million tokens times tokens, in micro-dollars.
pub fn estimate_micros(
    input_tokens: u64,
    output_tokens: u64,
    input_micros_per_million: u64,
    output_micros_per_million: u64,
) -> u64 {
    let input = u128::from(input_tokens) * u128::from(input_micros_per_million);
    let output = u128::from(output_tokens) * u128::from(output_micros_per_million);
    u64::try_from(input.saturating_add(output).div_ceil(1_000_000)).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dollars_keep_two_significant_digits() {
        for (micros, expected) in [
            (0, "0"),
            (1, "0.000001"),
            (4_000, "0.004"),
            (4_321, "0.0043"),
            (120_000, "0.12"),
            (999_999, "1"),
            (1_234_567, "1.23"),
            (u64::MAX, "18446744073709.55"),
        ] {
            assert_eq!(usd(micros), expected, "{micros}");
        }
    }

    #[test]
    fn estimates_round_up() {
        assert_eq!(estimate_micros(1_000, 500, 150_000, 600_000), 450);
        assert_eq!(estimate_micros(1, 0, 1, 0), 1);
        assert_eq!(
            estimate_micros(u64::MAX, u64::MAX, u64::MAX, u64::MAX),
            u64::MAX
        );
    }
}
