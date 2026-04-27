use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("not yet implemented")]
    NotImplemented,
}
