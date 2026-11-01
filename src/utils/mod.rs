use std::ffi::OsString;
use std::path::PathBuf;

/// Extracts the single input path from the program arguments.
pub fn input_path(args: impl Iterator<Item = OsString>) -> Option<PathBuf> {
    let mut args = args.skip(1);
    let path = args.next()?;
    if args.next().is_some() {
        return None;
    }
    Some(PathBuf::from(path))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::input_path;

    fn args(values: &[&str]) -> impl Iterator<Item = OsString> {
        values
            .iter()
            .map(OsString::from)
            .collect::<Vec<_>>()
            .into_iter()
    }

    #[test]
    fn single_argument_is_the_input_path() {
        let path = input_path(args(&["toymachine", "transactions.csv"]));

        assert_eq!(path, Some(PathBuf::from("transactions.csv")));
    }

    #[test]
    fn program_name_alone_is_rejected() {
        assert_eq!(input_path(args(&["toymachine"])), None);
    }

    #[test]
    fn no_arguments_at_all_is_rejected() {
        assert_eq!(input_path(args(&[])), None);
    }

    #[test]
    fn extra_arguments_are_rejected() {
        assert_eq!(input_path(args(&["toymachine", "a.csv", "b.csv"])), None);
    }
}
