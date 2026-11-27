//! Versions as they compare, and the version a caller names in its `User-Agent`.

use fearlezz_proof::version::Version;

#[test]
fn fewer_parts_are_zeros() {
    assert_eq!(Version::parse("1.0"), Version::parse("1.0.0"));
    assert_eq!(Version::parse("1"), Version::parse("1.0.0"));
    assert!(Version::parse("1.0") < Version::parse("1.0.1"));
    assert!(Version::parse("0.10.0") > Version::parse("0.9.9"));
}

#[test]
fn a_prerelease_comes_before_its_release() {
    assert!(Version::parse("0.6.0-rc.1") < Version::parse("0.6.0"));
    assert!(Version::parse("0.6.0-rc.1") > Version::parse("0.5.9"));
    assert_eq!(Version::parse("0.6.0+build"), Version::parse("0.6.0"));
    assert_eq!(Version::parse("v0.6.0"), Version::parse("0.6.0"));
}

#[test]
fn a_caller_names_its_version_after_fearlezz() {
    assert_eq!(Version::of_agent("fearlezz/0.5.4"), Version::parse("0.5.4"));
    assert_eq!(
        Version::of_agent("curl/8.0 fearlezz/1.2.3"),
        Version::parse("1.2.3")
    );
}

#[test]
fn a_caller_that_names_no_version_counts_as_the_oldest() {
    let oldest = Version::parse("0.0.0");
    assert_eq!(Version::of_agent("fearlezz"), oldest);
    assert_eq!(fearlezz_proof::version::named("fearlezz/"), None);
    assert_eq!(Version::of_agent("ureq/3.0"), oldest);
    assert_eq!(Version::of_agent(""), oldest);
    assert!(Version::of_agent("fearlezz") < Version::parse("0.0.1"));
}
