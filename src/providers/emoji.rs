//! Emoji picker (":" mode): search by Japanese or English name, Enter
//! pastes the emoji into the previous window.

use super::{Action, Item};
use std::sync::OnceLock;

/// emoji \t ja name \t en name \t keywords (see the file header).
const DATA: &str = include_str!("../../assets/emoji.tsv");

/// Parsed on first use only; the table stays out of memory until then.
pub fn items() -> &'static [Item] {
    static ITEMS: OnceLock<Vec<Item>> = OnceLock::new();
    ITEMS.get_or_init(|| {
        let mut items: Vec<Item> = DATA
            .lines()
            .filter(|l| !l.starts_with('#') && !l.is_empty())
            .filter_map(|line| {
                let mut f = line.split('\t');
                let (emoji, ja, en) = (f.next()?, f.next()?, f.next()?);
                let words = f.next().unwrap_or("");
                let mut item = Item::new(
                    format!("{emoji}  {ja}"),
                    format!("{en}  (Enter で貼り付け)"),
                    Action::Paste(emoji.to_string()),
                );
                item.key = format!("{ja} {en} {words}");
                item.extra = vec![("コピーだけする".into(), Action::Copy(emoji.to_string()))];
                Some(item.transient())
            })
            .collect();
        crate::reading::annotate_kana(&mut items);
        items
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_loads() {
        let items = items();
        assert!(items.len() > 1000);
        let cat = items.iter().find(|it| it.key.contains("cat")).unwrap();
        assert!(matches!(&cat.action, Action::Paste(e) if !e.is_empty()));
    }
}
