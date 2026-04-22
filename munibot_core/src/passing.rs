/// A trait that is intended to allow errors (in, i.e., Result types) to
/// "pass". Calling `pass` consumes the implementing type and, if applicable,
/// handles errors if present without panicking or diverting flow. This allows
/// errors to be seen without stopping program execution and without errors
/// being ignored completely.
pub trait Passing {
    fn pass(self);
}

impl<T, E: std::error::Error> Passing for Result<T, E> {
    /// Consume this Result and log the error with `log::error!`, if present.
    fn pass(self) {
        if let Err(e) = self {
            log::error!("{e}");
        }
    }
}
