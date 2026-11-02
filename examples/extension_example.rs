use serde_json::{Value, json};
use std::io;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let request: Value = serde_json::from_reader(io::stdin())?;
    let provider = &request["provider"];
    let contribution = match request["capability"].as_str() {
        Some("classify") => {
            let proposals: Vec<_> = request["processes"].as_array().ok_or("missing processes")?
                .iter().filter_map(|process| {
                    let cwd = process["cwd"].as_str()?;
                    std::path::Path::new(cwd).join(".reswho-example").is_file().then(|| json!({
                        "identity": {"pid":process["pid"],"started_unix_us":process["started_unix_us"]},
                        "evidence":{"kind":"custom","key":"marked-workspace","label":"Example workspace","path":cwd,"basis":"cwd"}
                    }))
                }).collect();
            json!({"capability":"classify","proposals":proposals})
        }
        Some("details") => json!({
            "capability":"parsed_details",
            "identity":{"pid":request["process"]["pid"],"started_unix_us":request["process"]["started_unix_us"]},
            "fields":[
                {"key":format!("extensions.{}.sample",provider.as_str().ok_or("provider")?),"label":"Shared sample ms","value":request["settings"]["sampling.interval_ms"],"sensitive":false},
                {"key":format!("extensions.{}.host",provider.as_str().ok_or("provider")?),"label":"Host CLI version","value":request["host"]["version"],"sensitive":false},
                {"key":format!("extensions.{}.token",provider.as_str().ok_or("provider")?),"label":"Example token","value":"fixture-secret-value","sensitive":false}
            ]
        }),
        _ => return Err("unsupported capability".into()),
    };
    serde_json::to_writer(
        io::stdout(),
        &json!({"protocol_version":1,"provider":provider,"version":request["version"],"contribution":contribution}),
    )?;
    Ok(())
}
