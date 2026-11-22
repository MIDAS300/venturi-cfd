use thiserror::Error;

#[derive(Debug, Error)]
pub enum VenturiError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Invalid STL file: {0}")]
    InvalidStl(String),

    #[error("Invalid OpenFOAM case: {0}")]
    InvalidCase(String),

    #[error("Physics parameter out of physical bounds: {0}")]
    PhysicsBounds(String),

    #[error("Solver execution error: {0}")]
    Execution(String),
}

impl From<VenturiError> for String {
    fn from(err: VenturiError) -> Self {
        err.to_string()
    }
}
