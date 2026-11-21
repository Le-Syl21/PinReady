//! Keyboard mnemonics, for cabinets without a mouse: a button shows one
//! underlined letter and Alt + that letter presses it.
//!
//! Alt + letter is the usual mnemonic chord and no desktop takes it for itself
//! (GNOME, KDE and Windows keep Alt+Tab, Alt+F1..F10, Alt+F4, Alt+Space; macOS
//! keeps ⌘). Ctrl is avoided on purpose: Ctrl+C copies, and the cabinet's magna
//! buttons are the Ctrl keys. Alt is only taken without Ctrl, because Windows
//! reports AltGr as Ctrl+Alt (AltGr+E is €), so typing is never disturbed. On
//! macOS Option + letter types a character (ç…): it is swallowed with the
//! chord so the launcher's type-anywhere search does not receive it.
//!
//! The letter comes from the translated label: the first letter not used yet
//! by another button of the same screen (accents ignored). A label without a
//! Latin letter (Cyrillic, CJK, Arabic…) gets the English label's letter in
//! brackets after it, as Windows does: "設定 (C)".

/// One frame's mnemonics on one screen.
pub struct Mnemonics {
    used: Vec<char>,
    /// Letters (A-Z) and digits pressed with Alt this frame.
    pressed: Vec<char>,
}

/// A label with its mnemonic letter, if one was free.
pub struct Mnemonic {
    pub text: String,
    /// Char index of the underlined letter in `text`.
    pub underline: Option<usize>,
    pub key: Option<char>,
}

/// The ASCII letter a character stands for (accents dropped), uppercase.
fn base_letter(c: char) -> Option<char> {
    let c = match c {
        'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'À' | 'Á' | 'Â' | 'Ã' | 'Ä' | 'Å' => 'a',
        'ç' | 'Ç' | 'č' | 'Č' => 'c',
        'è' | 'é' | 'ê' | 'ë' | 'È' | 'É' | 'Ê' | 'Ë' | 'ě' | 'Ě' => 'e',
        'ì' | 'í' | 'î' | 'ï' | 'Ì' | 'Í' | 'Î' | 'Ï' => 'i',
        'ñ' | 'Ñ' | 'ň' | 'Ň' => 'n',
        'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'Ò' | 'Ó' | 'Ô' | 'Õ' | 'Ö' => 'o',
        'ù' | 'ú' | 'û' | 'ü' | 'Ù' | 'Ú' | 'Û' | 'Ü' | 'ů' | 'Ů' => 'u',
        'ý' | 'ÿ' | 'Ý' => 'y',
        'š' | 'Š' | 'ś' | 'Ś' => 's',
        'ž' | 'Ž' | 'ź' | 'Ź' | 'ż' | 'Ż' => 'z',
        'ř' | 'Ř' => 'r',
        'ł' | 'Ł' => 'l',
        'ğ' | 'Ğ' => 'g',
        c => c,
    };
    c.is_ascii_alphabetic().then(|| c.to_ascii_uppercase())
}

impl Mnemonics {
    /// Take this frame's Alt + letter / digit presses out of the input, before
    /// the launcher navigation and the search read it.
    pub fn begin(ctx: &egui::Context) -> Self {
        let mut pressed = Vec::new();
        ctx.input_mut(|i| {
            let chord = |m: &egui::Modifiers| m.alt && !m.ctrl && !m.mac_cmd;
            let alt_held = chord(&i.modifiers);
            i.events.retain(|e| match e {
                egui::Event::Key {
                    key,
                    pressed: down,
                    modifiers,
                    ..
                } if chord(modifiers) => {
                    let name = key.name();
                    let c = name.chars().next().filter(|_| name.chars().count() == 1);
                    match c.filter(|c| c.is_ascii_alphanumeric()) {
                        Some(c) => {
                            if *down {
                                pressed.push(c.to_ascii_uppercase());
                            }
                            false
                        }
                        None => true,
                    }
                }
                // what Option + letter typed on macOS
                egui::Event::Text(_) if alt_held => false,
                _ => true,
            });
        });
        Self {
            used: Vec::new(),
            pressed,
        }
    }

    /// Give `text` the first free letter; `english` supplies one when `text`
    /// has no Latin letter.
    pub fn pick(&mut self, text: &str, english: &str) -> Mnemonic {
        for (idx, c) in text.chars().enumerate() {
            if let Some(b) = base_letter(c)
                && !self.used.contains(&b)
            {
                self.used.push(b);
                return Mnemonic {
                    text: text.to_string(),
                    underline: Some(idx),
                    key: Some(b),
                };
            }
        }
        let has_latin = text.chars().any(|c| base_letter(c).is_some());
        if !has_latin
            && let Some(b) = english
                .chars()
                .filter_map(base_letter)
                .find(|b| !self.used.contains(b))
        {
            self.used.push(b);
            let text = format!("{text} ({b})");
            let idx = text.chars().count() - 2;
            return Mnemonic {
                text,
                underline: Some(idx),
                key: Some(b),
            };
        }
        Mnemonic {
            text: text.to_string(),
            underline: None,
            key: None,
        }
    }

    /// Alt + this mnemonic's letter was pressed this frame.
    pub fn hit(&self, m: &Mnemonic) -> bool {
        m.key.is_some_and(|k| self.pressed.contains(&k))
    }

    /// Alt + this digit was pressed this frame.
    pub fn digit(&self, d: u32) -> bool {
        char::from_digit(d, 10).is_some_and(|c| self.pressed.contains(&c))
    }
}

impl Mnemonic {
    /// The label as button text, its letter underlined.
    pub fn widget_text(
        &self,
        ui: &egui::Ui,
        color: egui::Color32,
        size: Option<f32>,
    ) -> egui::WidgetText {
        let mut font = egui::TextStyle::Button.resolve(ui.style());
        if let Some(s) = size {
            font.size = s;
        }
        let mut job = egui::text::LayoutJob::default();
        let plain = egui::TextFormat {
            font_id: font.clone(),
            color,
            ..Default::default()
        };
        let mut underlined = plain.clone();
        underlined.underline = egui::Stroke::new((font.size / 14.0).max(1.0), color);
        for (i, c) in self.text.chars().enumerate() {
            let fmt = if Some(i) == self.underline {
                underlined.clone()
            } else {
                plain.clone()
            };
            job.append(&c.to_string(), 0.0, fmt);
        }
        job.into()
    }

    /// "Alt+C", for a tooltip.
    pub fn hint(&self) -> Option<String> {
        self.key.map(|k| format!("Alt+{k}"))
    }

    /// The button's response with its "Alt+C" tooltip.
    pub fn tooltip(&self, r: egui::Response) -> egui::Response {
        match self.hint() {
            Some(h) => r.on_hover_text(h),
            None => r,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn picker() -> Mnemonics {
        Mnemonics {
            used: Vec::new(),
            pressed: Vec::new(),
        }
    }

    #[test]
    fn first_free_letter_accents_ignored() {
        let mut m = picker();
        let a = m.pick("Configuration", "Configuration");
        assert_eq!((a.key, a.underline), (Some('C'), Some(0)));
        let b = m.pick("Écran", "Screen");
        // É stands for E, still free
        assert_eq!((b.key, b.underline), (Some('E'), Some(0)));
        let c = m.pick("Quitter", "Quit");
        assert_eq!(c.key, Some('Q'));
        let d = m.pick("Continuer", "Continue");
        // C taken: the next free letter of the label
        assert_eq!((d.key, d.underline), (Some('O'), Some(1)));
    }

    #[test]
    fn label_without_latin_letter_borrows_the_english_one() {
        let mut m = picker();
        let a = m.pick("設定", "Configuration");
        assert_eq!(a.text, "設定 (C)");
        assert_eq!(a.key, Some('C'));
        assert_eq!(a.text.chars().nth(a.underline.unwrap()), Some('C'));
    }

    #[test]
    fn arrows_and_symbols_are_skipped() {
        let mut m = picker();
        let a = m.pick("\u{2190}  Précédent", "Previous");
        assert_eq!((a.key, a.underline), (Some('P'), Some(3)));
    }
}
