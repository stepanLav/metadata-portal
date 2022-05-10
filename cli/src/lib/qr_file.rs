use anyhow::{bail, Context};
use std::convert::TryFrom;
use std::fmt;
use std::fmt::{Debug, Display, Formatter};
use std::path::PathBuf;

#[derive(Debug, PartialEq, Clone, Hash, Eq)]
struct QrPath<T: QrType> {
    pub dir: PathBuf,
    pub file_name: QrFileName<T>,
}

impl<T: QrType> QrPath<T> {
    pub fn to_path_buf(&self) -> PathBuf {
        self.dir.join(&self.file_name.to_string())
    }
}

impl<T: QrType> TryFrom<&PathBuf> for QrPath<T> {
    type Error = anyhow::Error;

    fn try_from(path: &PathBuf) -> Result<Self, Self::Error> {
        Ok(Self {
            dir: path.parent().unwrap().to_path_buf(),
            file_name: QrFileName::try_from(path)?,
        })
    }
}

impl<T: QrType> fmt::Display for QrPath<T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "{}",
            self.dir.join(&self.file_name.to_string()).to_str().unwrap()
        )
    }
}

#[derive(Debug, PartialEq, Clone, Hash, Eq)]
struct Metadata {
    pub version: u32,
}
#[derive(Debug, PartialEq, Clone, Hash, Eq)]
struct Specs {}

trait QrType: TryFrom<String> + Display {}

impl QrType for Metadata {}
impl QrType for Specs {}

impl fmt::Display for Specs {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "specs")
    }
}

impl fmt::Display for Metadata {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "metadata_{}", self.version)
    }
}

impl TryFrom<String> for Metadata {
    type Error = anyhow::Error;

    fn try_from(content_type: String) -> Result<Self, Self::Error> {
        let mut split = content_type.split('_');
        if let (Some("metadata"), Some(version)) = (split.next(), split.next()) {
            if let Ok(version) = version.parse::<u32>() {
                return Ok(Self { version });
            }
        }
        bail!("invalid content type: {}", content_type);
    }
}

impl TryFrom<String> for Specs {
    type Error = anyhow::Error;

    fn try_from(content_type: String) -> Result<Self, Self::Error> {
        if let "specs" = content_type.as_str() {
            return Ok(Self {});
        }
        bail!("unable to parse content type {}", content_type)
    }
}

#[derive(Debug, PartialEq, Clone, Hash, Eq)]
struct QrFileName<T: QrType> {
    pub chain: String,
    pub is_signed: bool,
    pub content_type: T,
    extension: Option<String>,
}

impl<T: QrType> QrFileName<T> {
    const UNSIGNED_PREFIX: &'static str = "unsigned_";
}

impl QrFileName<Metadata> {
    pub fn new(chain: &str, version: u32, is_signed: bool) -> Self {
        QrFileName {
            chain: chain.to_owned(),
            content_type: Metadata { version },
            is_signed,
            extension: Some("apng".to_string()),
        }
    }
}

impl QrFileName<Specs> {
    pub fn new(chain: &str, is_signed: bool) -> Self {
        QrFileName {
            chain: chain.to_owned(),
            content_type: Specs {},
            is_signed,
            extension: Some("png".to_string()),
        }
    }
}

impl<T: QrType> TryFrom<&PathBuf> for QrFileName<T> {
    type Error = anyhow::Error;

    fn try_from(path: &PathBuf) -> Result<Self, Self::Error> {
        let extension = path.extension().map(|s| s.to_str().unwrap().to_owned());
        let filename = path.file_stem().unwrap().to_str().unwrap();

        let (stripped, is_signed) = match filename.strip_prefix(QrFileName::<T>::UNSIGNED_PREFIX) {
            Some(s) => (s, false),
            None => (filename, true),
        };

        let mut split = stripped.splitn(2, '_');
        let chain = split.next().context("error parsing chain name")?;
        let content_type = split.next().context("error parsing context type")?;

        let content_type =
            T::try_from(content_type.to_string()).map_err(|e| anyhow::anyhow!("cannot convert"))?;
        Ok(Self {
            chain: String::from(chain),
            content_type,
            is_signed,
            extension,
        })
    }
}

impl<T: QrType> fmt::Display for QrFileName<T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let prefix = match self.is_signed {
            false => QrFileName::<T>::UNSIGNED_PREFIX,
            true => "",
        };
        let file_name = format!("{}{}_{}", prefix, self.chain, self.content_type);
        match &self.extension {
            Some(ext) => write!(f, "{}.{}", file_name, ext),
            None => write!(f, "{}", file_name),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Error;

    #[test]
    fn parse_valid_metadata_qr_path() {
        let path = PathBuf::from("./foo/bar/name_metadata_9123.apng");
        let result = QrPath::try_from(&path);
        assert!(result.is_ok());
        let parsed = result.unwrap();
        assert_eq!(parsed.dir, PathBuf::from("./foo/bar/"));
        assert_eq!(
            parsed.file_name,
            QrFileName::<Metadata>::new("name", 9123, true)
        );
        assert_eq!(parsed.to_path_buf(), path)
    }

    #[test]
    fn parse_unsigned_metadata_qr() {
        let path = PathBuf::from("./foo/bar/unsigned_polkadot_metadata_9123.apng");
        let parse_result: Result<QrFileName<Metadata>, Error> = QrFileName::try_from(&path);
        assert!(parse_result.is_ok());
        assert_eq!(
            parse_result.unwrap(),
            QrFileName::<Metadata>::new("polkadot", 9123, false)
        )
    }

    #[test]
    fn parse_invalid_filename() {
        let path = PathBuf::from("./foo/bar/invalid_9123.apng");
        let parse_result = QrFileName::<Specs>::try_from(&path);
        assert!(parse_result.is_err());
    }

    #[test]
    fn qr_signed_metadata_to_string() {
        let obj = QrFileName::<Metadata>::new("chain", 9000, true);
        assert_eq!(obj.to_string(), "chain_metadata_9000.apng");
    }

    #[test]
    fn qr_unsigned_to_string() {
        let obj = QrFileName::<Metadata>::new("chain", 9000, false);
        assert_eq!(obj.to_string(), "unsigned_chain_metadata_9000.apng");
    }

    #[test]
    fn parse_specs_qr_path() {
        let path = PathBuf::from("./foo/bar/polkadot_specs.png");
        let result = QrPath::try_from(&path);
        assert!(result.is_ok());
        let parsed = result.unwrap();
        assert_eq!(parsed.dir, PathBuf::from("./foo/bar/"));
        assert_eq!(parsed.file_name, QrFileName::<Specs>::new("polkadot", true));
        assert_eq!(parsed.to_path_buf(), path)
    }

    #[test]
    fn parse_specs_qr_to_string() {
        let obj = QrFileName::<Specs>::new("polkadot", true);
        assert_eq!(obj.to_string(), "polkadot_specs.png");
    }

    #[test]
    fn parse_unsigned_specs_qr_to_string() {
        let obj = QrFileName::<Specs>::new("polkadot", false);
        assert_eq!(obj.to_string(), "unsigned_polkadot_specs.png");
    }
}
