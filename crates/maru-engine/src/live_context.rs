//! Completed subtitle turns stay in capture order when translation requests overlap.

#[derive(Default)]
pub(crate) struct LiveContext {
    turns: Vec<Turn>,
}

struct Turn {
    id: u64,
    source: String,
    translation: String,
}

impl LiveContext {
    pub(crate) fn clear(&mut self) {
        self.turns.clear();
    }

    pub(crate) fn before(&self, id: u64) -> Vec<(String, String)> {
        self.turns
            .iter()
            .filter(|turn| turn.id < id)
            .map(|turn| (turn.source.clone(), turn.translation.clone()))
            .collect()
    }

    pub(crate) fn remember(&mut self, id: u64, source: &str, translation: &str, cap: usize) {
        if cap == 0 || translation.trim().is_empty() {
            return;
        }
        let turn = Turn {
            id,
            source: source.into(),
            translation: translation.into(),
        };
        match self.turns.binary_search_by_key(&id, |turn| turn.id) {
            Ok(index) => self.turns[index] = turn,
            Err(index) => self.turns.insert(index, turn),
        }
        // Drop a block at once so the prompt prefix stays reusable for several requests.
        let step = (cap / 2).max(1);
        if self.turns.len() > cap.saturating_add(step) {
            self.turns.drain(..step);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn answers_that_finish_out_of_order_are_read_in_capture_order() {
        let mut context = LiveContext::default();
        for id in [3, 1, 2] {
            context.remember(id, &id.to_string(), "answer", 4);
        }
        assert_eq!(
            context
                .before(4)
                .iter()
                .map(|(source, _)| source.as_str())
                .collect::<Vec<_>>(),
            ["1", "2", "3"]
        );
        assert_eq!(context.before(2), [("1".into(), "answer".into())]);
        assert!(
            context.before(1).is_empty(),
            "later captures cannot become an earlier line's context"
        );
    }

    #[test]
    fn a_late_old_answer_cannot_displace_newer_turns_when_the_window_is_trimmed() {
        let mut context = LiveContext::default();
        for id in [2, 3, 4, 1, 5] {
            context.remember(id, &id.to_string(), "answer", 2);
        }
        assert_eq!(
            context
                .before(6)
                .iter()
                .map(|(source, _)| source.as_str())
                .collect::<Vec<_>>(),
            ["3", "4", "5"]
        );
        context.clear();
        assert!(context.before(7).is_empty());
    }
}
