#![cfg(feature = "jiff")]

use roundhouse::doc::ApiSchema;

fn format_of<T: ApiSchema>() -> String {
    let schema = serde_json::to_value(T::schema()).unwrap();
    schema["format"].as_str().unwrap().to_string()
}

#[test]
fn jiff_types_map_to_schema_formats() {
    assert_eq!(format_of::<jiff::Timestamp>(), "date-time");
    assert_eq!(format_of::<jiff::Zoned>(), "date-time");
    assert_eq!(format_of::<jiff::civil::Date>(), "date");
    assert_eq!(format_of::<jiff::civil::DateTime>(), "date-time");
    assert_eq!(format_of::<jiff::civil::Time>(), "time");
    assert_eq!(format_of::<jiff::Span>(), "duration");
    assert_eq!(format_of::<jiff::SignedDuration>(), "duration");
}
