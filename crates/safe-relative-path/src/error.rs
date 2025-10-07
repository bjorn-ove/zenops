#[derive(Debug, thiserror::Error, PartialEq)]
pub enum Error {
    #[error("The specified relative path {0:?} goes outside of the parent directory")]
    PathGoesOutsideParent(relative_path::RelativePathBuf),
}
