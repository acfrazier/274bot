//! S1 ships an explicit no-op provisioner. S4 lands the real one.

pub enum Provisioner {
    None,
}

impl Provisioner {
    pub fn check(&self) -> Option<()> {
        match self {
            Self::None => None,
        }
    }

    pub fn retreat(&self) {}
}
