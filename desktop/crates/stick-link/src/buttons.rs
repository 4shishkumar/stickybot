//! Device-wide gesture identifiers and the two physical actions handled by the Mac.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MacAction<'a> {
    App(&'a str),
    Quick(&'a str),
}

impl<'a> MacAction<'a> {
    pub fn parse(gesture: &str, action: &'a str) -> Option<Self> {
        if !matches!(gesture, "b2" | "b3" | "ab") {
            return None;
        }
        let (kind, name) = action.split_once(':')?;
        if name.is_empty()
            || name.trim_matches(' ') != name
            || name.len() > 32
            || name.chars().any(|c| c.is_control() || matches!(c, '/' | '\\'))
        {
            return None;
        }
        match kind {
            "app" => Some(Self::App(name)),
            "quick" => Some(Self::Quick(name)),
            _ => None,
        }
    }
}
