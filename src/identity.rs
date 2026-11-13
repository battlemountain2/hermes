pub(crate) const NAME: &str = "Hermes";
pub(crate) const APPLICATION_ID: &str = "io.github.battlemountain2.Hermes";
pub(crate) const DEVELOPMENT_ID: &str = "io.github.battlemountain2.Hermes.Dev";
pub(crate) const ICON_NAME: &str = APPLICATION_ID;

pub(crate) fn application_id(development: bool) -> &'static str {
    if development {
        DEVELOPMENT_ID
    } else {
        APPLICATION_ID
    }
}

#[cfg(test)]
mod tests;
