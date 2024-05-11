//!

macro_rules! key {
    (@name $($modifier:ident)|+ - $name:ident) => {
        // The key `$modifier` is expected to be one of
        // {SHIFT, CONTROL, ALT, SUPER, HYPER, META, NONE}
        KeyEvent {
            modifiers: $(KeyModifiers::$modifier)|+,
            code: KeyCode::$name,
            kind: KeyEventKind::Press,
            state: _,
        }
    };
    (@name $name:ident) => {
        KeyEvent {
            modifiers: KeyModifiers::NONE,
            code: KeyCode::$name,
            kind: KeyEventKind::Press,
            state: _,
        }
    };
    ($($modifier:ident)|+ - $char:expr) => {
        // The key `$modifier` is expected to be one of
        // {SHIFT, CONTROL, ALT, SUPER, HYPER, META, NONE}
        KeyEvent {
            modifiers: $(KeyModifiers::$modifier)|+,
            code: KeyCode::Char($char),
            kind: KeyEventKind::Press,
            state: _,
        }
    };
    ($($modifier:ident)|+ - @$char:ident) => {
        KeyEvent {
            modifiers: $(KeyModifiers::$modifier)|+,
            code: KeyCode::Char($char),
            kind: KeyEventKind::Press,
            state: _,
        }
    };
    (@$char:ident) => {
        KeyEvent {
            modifiers: KeyModifiers::NONE,
            code: KeyCode::Char($char),
            kind: KeyEventKind::Press,
            state: _,
        }
    };
    ($char:expr) => {
        KeyEvent {
            modifiers: KeyModifiers::NONE,
            code: KeyCode::Char($char),
            kind: KeyEventKind::Press,
            state: _,
        }
    };
}

pub(crate) use key;
