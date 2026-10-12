/// Profile-wide reading choices, independent of a publication and its progress.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ReaderSettings {
    pub fit: ReaderFit,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ReaderFit {
    #[default]
    Height,
    Width,
}
