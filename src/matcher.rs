use crate::providers::Item;
use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};

pub struct Ranker {
    matcher: Matcher,
}

impl Ranker {
    pub fn new() -> Self {
        Self {
            matcher: Matcher::new(Config::DEFAULT),
        }
    }

    /// Returns indices into `items`, best match first, at most `max`.
    /// `bonus` adds an extra per-item score (e.g. launch-count history).
    pub fn rank(
        &mut self,
        items: &[Item],
        query: &str,
        max: usize,
        bonus: impl Fn(&Item) -> u32,
    ) -> Vec<usize> {
        let pattern = Pattern::parse(query, CaseMatching::Ignore, Normalization::Smart);
        let mut buf = Vec::new();
        let mut scored: Vec<(u32, usize)> = Vec::new();
        for (i, item) in items.iter().enumerate() {
            let hay = Utf32Str::new(&item.key, &mut buf);
            if let Some(score) = pattern.score(hay, &mut self.matcher) {
                scored.push((score + bonus(item), i));
            }
        }
        scored.sort_unstable_by(|a, b| b.0.cmp(&a.0));
        scored.truncate(max);
        scored.into_iter().map(|(_, i)| i).collect()
    }
}
