//! The shared, deliberately small chart contract: numeric line charts over local
//! CSV/JSON rows, optional nominal series, and optional absolute lower/upper
//! bounds. Options that would change interpretation must be implemented here and
//! in the SVG renderer together; accepting and ignoring them changes evidence.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use serde_json::Value;

use crate::deck::{ChartData, ChartDataFormat};

#[derive(Debug, Clone)]
pub(crate) struct ChartPoint {
    pub(crate) x: f64,
    pub(crate) y: f64,
    pub(crate) series: String,
    pub(crate) lower: Option<f64>,
    pub(crate) upper: Option<f64>,
}

pub(crate) struct LineChart {
    pub(crate) points: Vec<ChartPoint>,
    pub(crate) x_title: String,
    pub(crate) y_title: String,
    pub(crate) color_title: Option<String>,
}

struct Fields<'a> {
    x: &'a str,
    y: &'a str,
    color: Option<&'a str>,
    lower: Option<&'a str>,
    upper: Option<&'a str>,
    x_title: &'a str,
    y_title: &'a str,
    color_title: Option<&'a str>,
}

/// Source-only parsing can validate the specification without a Deck root.
/// With a root, validate every row as well, before a build can succeed.
pub(crate) fn validate(
    spec: &Value,
    data: Option<&ChartData>,
    deck_root: Option<&Path>,
) -> Result<(), String> {
    if deck_root.is_some() {
        load(spec, data, deck_root).map(|_| ())
    } else {
        fields(spec, data).map(|_| ())
    }
}

pub(crate) fn load(
    spec: &Value,
    data: Option<&ChartData>,
    deck_root: Option<&Path>,
) -> Result<LineChart, String> {
    let fields = fields(spec, data)?;
    let data = data.expect("validated chart has local data");
    let root = deck_root.ok_or("chart data cannot be resolved without a Deck root")?;
    let rows = match data.format {
        ChartDataFormat::Csv => csv_rows(&root.join(&data.url)),
        ChartDataFormat::Json => json_rows(&root.join(&data.url)),
    }
    .map_err(|reason| format!("chart data '{}': {reason}", data.url))?;
    let points =
        points(&rows, &fields).map_err(|reason| format!("chart data '{}': {reason}", data.url))?;
    Ok(LineChart {
        points,
        x_title: fields.x_title.to_string(),
        y_title: fields.y_title.to_string(),
        color_title: fields.color_title.map(str::to_string),
    })
}

fn only_keys(value: &Value, allowed: &[&str], context: &str) -> Result<(), String> {
    let object = value
        .as_object()
        .ok_or_else(|| format!("{context} must be an object"))?;
    for key in object.keys() {
        if !allowed.contains(&key.as_str()) {
            return Err(format!("unsupported chart option '{context}.{key}'"));
        }
    }
    Ok(())
}

fn fields<'a>(spec: &'a Value, data: Option<&ChartData>) -> Result<Fields<'a>, String> {
    only_keys(
        spec,
        &[
            "$schema",
            "description",
            "name",
            "usermeta",
            "data",
            "mark",
            "encoding",
        ],
        "spec",
    )?;
    let mark = &spec["mark"];
    let mark_type = mark
        .as_str()
        .or_else(|| mark["type"].as_str())
        .unwrap_or("");
    if mark_type != "line" {
        return Err(format!(
            "only Vega-Lite line charts have a reliable renderer, found '{mark_type}'"
        ));
    }
    if mark.is_object() {
        only_keys(mark, &["type", "point"], "mark")?;
        if mark
            .get("point")
            .is_some_and(|point| point != &Value::Bool(true))
        {
            return Err("mark.point supports only true; line charts include point marks".into());
        }
    }
    if let Some(data_spec) = spec.get("data") {
        only_keys(data_spec, &["url"], "data")?;
        if data_spec
            .get("url")
            .is_none_or(|url| url.as_str().is_none())
        {
            return Err("data.url must name a local CSV or JSON file".into());
        }
    }
    if data.is_none() {
        return Err("charts must resolve to local CSV or JSON data".into());
    }
    let encoding = &spec["encoding"];
    only_keys(
        encoding,
        &["x", "y", "color", "yError", "yError2"],
        "encoding",
    )?;
    let x = channel(encoding, "x", "quantitative", true)?;
    let y = channel(encoding, "y", "quantitative", true)?;
    let color = optional_channel(encoding, "color", "nominal", true)?;
    let lower = optional_channel(encoding, "yError", "quantitative", false)?;
    let upper = optional_channel(encoding, "yError2", "quantitative", false)?;
    if lower.is_some() != upper.is_some() {
        return Err("encoding.yError and encoding.yError2 must both declare absolute lower/upper bound fields".into());
    }
    Ok(Fields {
        x,
        y,
        color,
        lower,
        upper,
        x_title: encoding["x"]["title"].as_str().unwrap_or(x),
        y_title: encoding["y"]["title"].as_str().unwrap_or(y),
        color_title: color.map(|field| encoding["color"]["title"].as_str().unwrap_or(field)),
    })
}

fn optional_channel<'a>(
    encoding: &'a Value,
    name: &str,
    kind: &str,
    allow_title: bool,
) -> Result<Option<&'a str>, String> {
    encoding
        .get(name)
        .map(|_| channel(encoding, name, kind, allow_title))
        .transpose()
}

fn channel<'a>(
    encoding: &'a Value,
    name: &str,
    kind: &str,
    allow_title: bool,
) -> Result<&'a str, String> {
    let value = &encoding[name];
    let context = format!("encoding.{name}");
    let allowed = if allow_title {
        &["field", "type", "title"][..]
    } else {
        &["field", "type"][..]
    };
    only_keys(value, allowed, &context)?;
    let field = value["field"]
        .as_str()
        .filter(|field| !field.is_empty())
        .ok_or_else(|| format!("{context}.field must be a non-empty field name"))?;
    if value
        .get("type")
        .is_some_and(|value| value.as_str() != Some(kind))
    {
        return Err(format!("{context}.type supports only '{kind}'"));
    }
    if value
        .get("title")
        .is_some_and(|value| value.as_str().is_none())
    {
        return Err(format!("{context}.title must be a string"));
    }
    Ok(field)
}

type Row = BTreeMap<String, String>;

fn csv_rows(path: &Path) -> Result<Vec<Row>, String> {
    let mut reader = csv::ReaderBuilder::new()
        .trim(csv::Trim::All)
        .from_path(path)
        .map_err(|error| format!("could not read CSV: {error}"))?;
    let headers = reader
        .headers()
        .map_err(|error| format!("invalid CSV header: {error}"))?
        .clone();
    let mut seen = BTreeSet::new();
    for header in &headers {
        if header.is_empty() || !seen.insert(header) {
            return Err(format!(
                "CSV headers must be non-empty and unique (found '{header}')"
            ));
        }
    }
    reader
        .records()
        .enumerate()
        .map(|(index, record)| {
            let record = record.map_err(|error| format!("CSV data row {}: {error}", index + 1))?;
            Ok(headers
                .iter()
                .zip(record.iter())
                .map(|(key, value)| (key.to_string(), value.to_string()))
                .collect())
        })
        .collect()
}

fn json_rows(path: &Path) -> Result<Vec<Row>, String> {
    let bytes = fs::read(path).map_err(|error| format!("could not read JSON: {error}"))?;
    let value: Value =
        serde_json::from_slice(&bytes).map_err(|error| format!("invalid JSON: {error}"))?;
    let rows = value
        .as_array()
        .ok_or("JSON data must be an array of objects")?;
    rows.iter()
        .enumerate()
        .map(|(index, value)| {
            let object = value
                .as_object()
                .ok_or_else(|| format!("data row {} must be an object", index + 1))?;
            Ok(object
                .iter()
                .filter_map(|(key, value)| {
                    let text = match value {
                        Value::String(text) => text.clone(),
                        Value::Number(_) | Value::Bool(_) => value.to_string(),
                        _ => return None,
                    };
                    Some((key.clone(), text))
                })
                .collect())
        })
        .collect()
}

fn points(rows: &[Row], fields: &Fields<'_>) -> Result<Vec<ChartPoint>, String> {
    if rows.is_empty() {
        return Err("chart has no data rows".into());
    }
    let points = rows
        .iter()
        .enumerate()
        .map(|(index, row)| {
            let value = |field: &str| {
                row.get(field).ok_or_else(|| {
                    format!(
                        "data row {}, field '{field}' is missing or is not a scalar value",
                        index + 1
                    )
                })
            };
            let number = |field: &str| {
                value(field)?
                    .parse::<f64>()
                    .ok()
                    .filter(|number| number.is_finite())
                    .ok_or_else(|| {
                        format!(
                            "data row {}, field '{field}' must be a finite number",
                            index + 1
                        )
                    })
            };
            let lower = fields.lower.map(number).transpose()?;
            let upper = fields.upper.map(number).transpose()?;
            if lower.zip(upper).is_some_and(|(lower, upper)| lower > upper) {
                return Err(format!(
                    "data row {} has a lower bound greater than its upper bound",
                    index + 1
                ));
            }
            Ok(ChartPoint {
                x: number(fields.x)?,
                y: number(fields.y)?,
                series: fields
                    .color
                    .map(value)
                    .transpose()?
                    .cloned()
                    .unwrap_or_else(|| "series".into()),
                lower,
                upper,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    finite_domain(points.iter().map(|point| point.x), fields.x)?;
    finite_domain(
        points.iter().flat_map(|point| {
            [Some(point.y), point.lower, point.upper]
                .into_iter()
                .flatten()
        }),
        fields.y,
    )?;
    Ok(points)
}

fn finite_domain(values: impl Iterator<Item = f64>, field: &str) -> Result<(), String> {
    let (mut min, mut max) = values
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(min, max), value| {
            (min.min(value), max.max(value))
        });
    if min == max {
        min -= 1.0;
        max += 1.0;
    }
    if !(max - min).is_finite() || max <= min {
        return Err(format!(
            "field '{field}' has a numeric range too large to render reliably"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn spec() -> Value {
        json!({
            "mark": "line",
            "encoding": {
                "x": {"field": "x", "type": "quantitative"},
                "y": {"field": "y", "type": "quantitative"}
            }
        })
    }

    fn load_data(
        spec: &Value,
        format: ChartDataFormat,
        contents: &str,
    ) -> Result<LineChart, String> {
        let temp = tempfile::tempdir().unwrap();
        let data = ChartData {
            url: "data".into(),
            format,
        };
        fs::write(temp.path().join(&data.url), contents).unwrap();
        load(spec, Some(&data), Some(temp.path()))
    }

    #[test]
    fn rejects_interpretation_options_instead_of_ignoring_them() {
        let data = ChartData {
            url: "data.csv".into(),
            format: ChartDataFormat::Csv,
        };
        for (pointer, value, diagnostic) in [
            (
                "/transform",
                json!([{"filter": "datum.x > 1"}]),
                "spec.transform",
            ),
            (
                "/encoding/x/scale",
                json!({"type": "log"}),
                "encoding.x.scale",
            ),
            (
                "/encoding/y/scale",
                json!({"domain": [0, 100]}),
                "encoding.y.scale",
            ),
            (
                "/encoding/y/aggregate",
                json!("mean"),
                "encoding.y.aggregate",
            ),
            ("/encoding/x/bin", json!(true), "encoding.x.bin"),
            ("/encoding/x/type", json!("temporal"), "encoding.x.type"),
            (
                "/encoding/color",
                json!({"field": "group", "type": "quantitative"}),
                "encoding.color.type",
            ),
            (
                "/encoding/order",
                json!({"field": "rank"}),
                "encoding.order",
            ),
            (
                "/mark",
                json!({"type": "line", "interpolate": "step"}),
                "mark.interpolate",
            ),
            (
                "/data",
                json!({"url": "data.csv", "format": {"parse": {"x": "date"}}}),
                "data.format",
            ),
        ] {
            let mut input = spec();
            let (parent, key) = pointer.rsplit_once('/').unwrap();
            input.pointer_mut(parent).unwrap()[key] = value;
            let error = validate(&input, Some(&data), None).unwrap_err();
            assert!(error.contains(diagnostic), "{pointer}: {error}");
        }
    }

    #[test]
    fn csv_preserves_quoted_fields_multiline_series_and_uncertainty() {
        let mut input = spec();
        input["mark"] = json!({"type": "line", "point": true});
        input["encoding"]["x"]["title"] = json!("Input size");
        input["encoding"]["y"]["title"] = json!("Runtime (ms)");
        input["encoding"]["color"] =
            json!({"field": "group", "type": "nominal", "title": "Method"});
        input["encoding"]["yError"] = json!({"field": "lower"});
        input["encoding"]["yError2"] = json!({"field": "upper"});
        let chart = load_data(&input, ChartDataFormat::Csv,
            "x,y,group,lower,upper\r\n\"1\",\"2.5\",\"A, \"\"quoted\"\"\",2,3\r\n2,4,\"B\nmultiline\",3,5\r\n").unwrap();
        assert_eq!(chart.points.len(), 2);
        assert_eq!(chart.points[0].series, "A, \"quoted\"");
        assert_eq!(chart.points[1].series, "B\nmultiline");
        assert_eq!((chart.points[0].x, chart.points[0].y), (1.0, 2.5));
        assert_eq!(
            (chart.points[1].lower, chart.points[1].upper),
            (Some(3.0), Some(5.0))
        );
        assert_eq!(
            (&*chart.x_title, &*chart.y_title),
            ("Input size", "Runtime (ms)")
        );
        assert_eq!(chart.color_title.as_deref(), Some("Method"));
    }

    #[test]
    fn rejects_csv_data_loss_with_row_and_field_context() {
        for (data, expected) in [
            (
                "x,y\n1,2\n3,no\n",
                "data row 2, field 'y' must be a finite number",
            ),
            (
                "x,y\n1,NaN\n",
                "data row 1, field 'y' must be a finite number",
            ),
            (
                "x,y\n1,inf\n",
                "data row 1, field 'y' must be a finite number",
            ),
            ("x,y\n1,\n", "data row 1, field 'y' must be a finite number"),
            ("x,y\n1,2,3\n", "CSV data row 1"),
            ("x,y\n1\n", "CSV data row 1"),
            ("x,x\n1,2\n", "headers must be non-empty and unique"),
            ("x,\n1,2\n", "headers must be non-empty and unique"),
            ("x,y\n", "no data rows"),
        ] {
            let error = load_data(&spec(), ChartDataFormat::Csv, data)
                .err()
                .unwrap();
            assert!(error.contains(expected), "{data:?}: {error}");
        }
    }

    #[test]
    fn json_requires_objects_and_valid_selected_fields_in_every_row() {
        let chart = load_data(
            &spec(),
            ChartDataFormat::Json,
            r#"[{"x":1,"y":"2.5","metadata":{"ignored":true}},{"x":2,"y":4}]"#,
        )
        .unwrap();
        assert_eq!(chart.points.len(), 2);
        assert_eq!(chart.points[0].y, 2.5);
        for (data, expected) in [
            (
                r#"[{"x":1,"y":2},{"x":2,"y":"invalid"}]"#,
                "data row 2, field 'y' must be a finite number",
            ),
            (
                r#"[{"x":1,"y":null}]"#,
                "data row 1, field 'y' is missing or is not a scalar",
            ),
            (r#"[{"x":1}]"#, "data row 1, field 'y' is missing"),
            (r#"[{"x":1,"y":"NaN"}]"#, "must be a finite number"),
            (r#"[{"x":1,"y":true}]"#, "must be a finite number"),
            (r#"[{"x":1,"y":2},7]"#, "data row 2 must be an object"),
            ("[]", "no data rows"),
        ] {
            let error = load_data(&spec(), ChartDataFormat::Json, data)
                .err()
                .unwrap();
            assert!(error.contains(expected), "{data}: {error}");
        }
    }

    #[test]
    fn rejects_missing_or_invalid_uncertainty_and_unrenderable_ranges() {
        let mut input = spec();
        input["encoding"]["yError"] = json!({"field": "lo"});
        assert!(
            load_data(&input, ChartDataFormat::Csv, "x,y,lo\n1,2,1\n")
                .err()
                .unwrap()
                .contains("must both declare")
        );
        input["encoding"]["yError2"] = json!({"field": "hi"});
        for (data, expected) in [
            (
                "x,y,lo,hi\n1,2,1,bad\n",
                "field 'hi' must be a finite number",
            ),
            ("x,y,lo,hi\n1,2,3,1\n", "lower bound greater than"),
            ("x,y,lo\n1,2,1\n", "field 'hi' is missing"),
        ] {
            assert!(
                load_data(&input, ChartDataFormat::Csv, data)
                    .err()
                    .unwrap()
                    .contains(expected)
            );
        }
        assert!(
            load_data(&spec(), ChartDataFormat::Csv, "x,y\n-1e308,1\n1e308,2\n")
                .err()
                .unwrap()
                .contains("numeric range too large")
        );
    }
}
