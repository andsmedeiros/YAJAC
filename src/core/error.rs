#[derive(Debug, Clone)]
pub enum Error {
    DataLoadingError {
        message: String
    }
}