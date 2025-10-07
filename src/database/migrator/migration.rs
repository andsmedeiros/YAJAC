use std::cmp::Ordering;

#[derive(Debug, Clone)]
pub struct Migration {
    pub version: usize,
    pub name: String,
    pub(super) up: String,
    pub(super) down: String,
}

impl PartialOrd for Migration {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(&other))
    }
}

impl Ord for Migration {
    fn cmp(&self, other: &Self) -> Ordering {
        self.version.cmp(&other.version)
    }
}

impl PartialEq for Migration {
    fn eq(&self, other: &Self) -> bool {
        self.version == other.version
    }
}

impl Eq for Migration {}


#[cfg(feature = "builtin_migrations")]
impl<'a> From<&include_dir::Dir<'a>> for Migration {
    fn from(dir: &Dir<'a>) -> Self {
        let path = dir.path().to_string_lossy();
        let (version, name) = path
            .split_once('-')
            .map(|(number, name)| (
                number.parse::<usize>().expect("Failed to parse migration number"),
                name.trim().to_string()
            ) )
            .expect("Unable to extract migration number and name from directory");

        let up = dir.get_file(dir.path().join("up.sql"))
            .expect(format!("Failed to get file 'up.sql' for migration {path}").as_str());
        let down = dir.get_file(dir.path().join("down.sql"))
            .expect(format!("Failed to get file 'down.sql' for migration {path}").as_str());

        Migration {
            version,
            name,
            up: String::from_utf8_lossy(up.contents()).to_string(),
            down: String::from_utf8_lossy(down.contents()).to_string()
        }
    }
}