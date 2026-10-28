const SIZES: &[(&str, [(u32, u32); 5])] = &[
    (
        "tiny",
        [
            (1025, 1025),
            (641, 1281),
            (513, 1537),
            (513, 1537),
            (385, 1921),
        ],
    ),
    (
        "small",
        [
            (2049, 2049),
            (1409, 2817),
            (1153, 3457),
            (1025, 4097),
            (897, 4481),
        ],
    ),
    (
        "medium",
        [
            (2817, 2817),
            (2049, 4097),
            (1665, 4993),
            (1537, 6145),
            (1281, 6401),
        ],
    ),
    (
        "large",
        [
            (3585, 3585),
            (2561, 5121),
            (2049, 6145),
            (1793, 7169),
            (1537, 8065),
        ],
    ),
    (
        "verylarge",
        [
            (4097, 4097),
            (2817, 5633),
            (2305, 6913),
            (2049, 8193),
            (1793, 8961),
        ],
    ),
    (
        "huge",
        [
            (5121, 5121),
            (3585, 7169),
            (2945, 8833),
            (2561, 10241),
            (2177, 10881),
        ],
    ),
    (
        "megalomaniac",
        [
            (6145, 6145),
            (4225, 8449),
            (3457, 10369),
            (3073, 12289),
            (2689, 13441),
        ],
    ),
    (
        "gigantomaniac",
        [
            (7169, 7169),
            (5121, 10241),
            (4097, 12289),
            (3585, 14337),
            (3201, 16001),
        ],
    ),
];

pub fn normalize_size(size: &str) -> Option<&'static str> {
    let s = size.to_ascii_lowercase().replace([' ', '_', '-'], "");
    SIZES.iter().map(|(name, _)| *name).find(|name| *name == s)
}

pub fn normalize_ratio(ratio: &str) -> Option<&'static str> {
    let r = ratio.replace(' ', "");
    match r.as_str() {
        "1:1" => Some("1:1"),
        "1:2" => Some("1:2"),
        "1:3" => Some("1:3"),
        "1:4" => Some("1:4"),
        "1:5" => Some("1:5"),
        _ => None,
    }
}

pub fn pixels(size: &str, ratio: &str) -> Option<(u32, u32)> {
    let size = normalize_size(size)?;
    let ratio = normalize_ratio(ratio)?;
    let index = match ratio {
        "1:1" => 0,
        "1:2" => 1,
        "1:3" => 2,
        "1:4" => 3,
        _ => 4,
    };
    SIZES
        .iter()
        .find(|(name, _)| *name == size)
        .map(|(_, dims)| dims[index])
}

pub fn display_size(size: &str) -> String {
    match normalize_size(size).unwrap_or("medium") {
        "tiny" => "Tiny".into(),
        "small" => "Small".into(),
        "medium" => "Medium".into(),
        "large" => "Large".into(),
        "verylarge" => "Very Large".into(),
        "huge" => "Huge".into(),
        "megalomaniac" => "Megalomaniac".into(),
        _ => "Gigantomaniac".into(),
    }
}

pub fn is_experimental(size: &str) -> bool {
    matches!(
        normalize_size(size),
        Some("tiny") | Some("huge") | Some("megalomaniac") | Some("gigantomaniac")
    )
}

pub fn list_sizes() -> String {
    SIZES
        .iter()
        .map(|(name, _)| *name)
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn medium_square_is_2817() {
        assert_eq!(pixels("medium", "1:1"), Some((2817, 2817)));
    }

    #[test]
    fn very_large_1_2() {
        assert_eq!(pixels("Very Large", "1:2"), Some((2817, 5633)));
    }

    #[test]
    fn gigantomaniac_1_5() {
        assert_eq!(pixels("gigantomaniac", "1:5"), Some((3201, 16001)));
    }

    #[test]
    fn dimensions_follow_four_metres_per_pixel() {
        for size in ["tiny", "small", "medium", "large", "verylarge"] {
            for ratio in ["1:1", "1:2", "1:3", "1:4", "1:5"] {
                let (w, h) = pixels(size, ratio).unwrap();
                assert!(w >= 257 && h >= 257);
                assert_eq!((w - 1) % 4, 0);
                assert_eq!((h - 1) % 4, 0);
            }
        }
    }
}
