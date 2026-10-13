use std::time::{Duration, Instant};

#[derive(Default)]
pub(crate) struct TextChange {
    candidate: Option<(String, Instant)>,
    accepted: Option<String>,
}

impl TextChange {
    pub fn waiting(&self) -> bool {
        self.candidate.is_some()
    }
    pub fn reset(&mut self) {
        *self = Self::default();
    }
    pub fn accept_manual(&mut self, text: &str) {
        self.candidate = None;
        self.accepted = Some(text.split_whitespace().collect::<Vec<_>>().join(" "));
    }
    /// Blank text is confirmed too, so a fleeting missed OCR frame cannot erase a reading.
    pub fn observe(&mut self, text: &str, now: Instant) -> Option<String> {
        let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
        if self.accepted.as_ref() == Some(&text) {
            self.candidate = None;
            return None;
        }
        match &self.candidate {
            Some((previous, since))
                if previous == &text
                    && now.duration_since(*since) >= Duration::from_millis(650) =>
            {
                self.accepted = Some(text.clone());
                self.candidate = None;
                Some(text)
            }
            Some((previous, _)) if previous == &text => None,
            _ => {
                self.candidate = Some((text, now));
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn typewriter_fragments_are_not_accepted_before_the_whole_text_settles() {
        let mut change = TextChange::default();
        let at = Instant::now();
        assert_eq!(change.observe("그때의", at), None);
        assert_eq!(
            change.observe("그때의 여행자들은", at + Duration::from_millis(600)),
            None
        );
        assert_eq!(
            change.observe("그때의 여행자들은", at + Duration::from_millis(1000)),
            None
        );
        assert_eq!(
            change.observe("그때의 여행자들은", at + Duration::from_millis(1300)),
            Some("그때의 여행자들은".into())
        );
    }
    #[test]
    fn background_changes_and_whitespace_do_not_repeat_an_accepted_paragraph() {
        let mut change = TextChange::default();
        let at = Instant::now();
        change.accept_manual("카메라 설정");
        assert_eq!(change.observe("카메라   설정\n", at), None);
        assert_eq!(change.observe("", at), None);
        assert_eq!(
            change.observe("", at + Duration::from_millis(700)),
            Some(String::new())
        );
        assert_eq!(
            change.observe("카메라 설정", at + Duration::from_millis(800)),
            None
        );
        assert_eq!(
            change.observe("카메라 설정", at + Duration::from_millis(1500)),
            Some("카메라 설정".into())
        );
    }
}
