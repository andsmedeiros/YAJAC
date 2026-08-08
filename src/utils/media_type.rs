/// A parsed media type: its essence (`type/subtype`, e.g. `application/vnd.api+json`) and its
/// parameters, each borrowed from the header they were parsed from.
#[derive(Debug, PartialEq, Eq)]
pub struct MediaType<'a> {
    pub essence: &'a str,
    pub parameters: Vec<(&'a str, &'a str)>,
}

impl<'a> MediaType<'a> {
    /// The value of a parameter, matched case-insensitively by name.
    pub fn parameter(&self, name: &str) -> Option<&'a str> {
        self.parameters
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| *value)
    }

    /// Parses a media-type header value into its media types: one for a `Content-Type`, several for a
    /// comma-separated `Accept`. Each instance splits into its essence and `name=value` parameters
    /// (surrounding quotes trimmed); an instance without an essence is dropped.
    pub fn list_from(header: &str) -> impl Iterator<Item = MediaType<'_>> {
        header.split(',').filter_map(|instance| {
            let mut parts = instance.split(';').map(str::trim);
            let essence = parts.next().filter(|essence| !essence.is_empty())?;
            let parameters = parts
                .filter_map(|parameter| parameter.split_once('='))
                .map(|(name, value)| (name.trim(), value.trim().trim_matches('"')))
                .collect();
            Some(MediaType {
                essence,
                parameters,
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::MediaType;

    #[test]
    fn parses_a_bare_media_type() {
        let types: Vec<_> = MediaType::list_from("application/vnd.api+json").collect();
        assert_eq!(
            types,
            vec![MediaType {
                essence: "application/vnd.api+json",
                parameters: vec![],
            }]
        );
    }

    #[test]
    fn parses_parameters_including_a_quoted_value() {
        let types: Vec<_> = MediaType::list_from(
            "application/vnd.api+json; charset=utf-8; profile=\"https://x/y\"",
        )
        .collect();
        assert_eq!(
            types[0].parameters,
            vec![("charset", "utf-8"), ("profile", "https://x/y"),]
        );
        assert_eq!(types[0].parameter("PROFILE"), Some("https://x/y"));
    }

    #[test]
    fn splits_a_comma_separated_accept_list() {
        let essences: Vec<_> = MediaType::list_from("text/html, application/vnd.api+json; ext=a")
            .map(|media| media.essence)
            .collect();
        assert_eq!(essences, vec!["text/html", "application/vnd.api+json"]);
    }
}
