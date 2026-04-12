/// A trait that is intended to allow errors (in, i.e., Result types) to "pass".
/// Calling `pass` consumes the implementing type and, if applicable, handles
/// errors if present without panicking or diverting flow. This allows errors to
/// be seen without stopping program execution and without errors being ignored
/// completely.
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

#[cfg(test)]
mod tests {
    use super::Passing;

    #[test]
    fn test_ok_does_not_panic() {
        // pass on Ok should be a no-op and never panic
        let result: Result<i32, std::io::Error> = Ok(42);
        result.pass(); // must not panic
    }

    #[test]
    fn test_err_does_not_panic() {
        // pass on Err should log the error but not panic
        let result: Result<(), std::io::Error> =
            Err(std::io::Error::new(std::io::ErrorKind::Other, "test error"));
        result.pass(); // must not panic
    }

    #[test]
    fn test_ok_consumes_value() {
        // pass consumes self, so this just needs to compile and not panic
        let result: Result<String, std::io::Error> = Ok("hello".to_string());
        result.pass();
    }
}
