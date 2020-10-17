extern crate reqwest;
extern crate tokio;
extern crate serde;
extern crate chrono;
extern crate confy;
extern crate log4rs;
extern crate directories;
#[macro_use] extern crate log;
use directories::ProjectDirs;
extern crate csv;
use chrono::{DateTime,Utc,Duration};
use serde::{Serialize,Deserialize};
use std::path::PathBuf;

const APPNAME: &'static str = "fema-web-declaration";
/*
 * Config file is placed in the expected place for the operating system using the mechanisms
 * Config file and log file are placed in the expected place for the operating system using the
 * mechanisms defined by:
    The XDG base directory and the XDG user directory specifications on Linux
        Config file defaults to $XDG_CONFIG_HOME/APPNAME/APPNAME.toml or ~/.config/APPNAME/APPNAME.toml (e.g. /home/thor/.config/APPNAME/APPNAME.toml )
    The Known Folder API on Windows
        Config file defaults to	{FOLDERID_RoamingAppData}\APPNAME\config\APPNAME.toml (e.g. C:\Users\thor\AppData\Roaming\APPNAME\config )
    The Standard Directories guidelines on macOS
        Config file defaults to	$HOME/Library/Application Support/APPNAME/APPNAME.toml (e.g. /Users/thor/Library/Application Support/APPNAME/APPNAME.toml
 */
#[derive(Debug,Serialize,Deserialize)]
struct Config {
    debug: bool,
    num_years_previous: usize,
    csv: Option<PathBuf>,
}
impl std::default::Default for Config {
    fn default() -> Self { Self { debug: false, num_years_previous: 3, csv: Some(PathBuf::from("out.csv"))}}
}

/* 
 * A couple of structs to define how to deserialize JSON results from the FEMA API
 * and serialize entries to be written to file if the csv option is enabled in the config file
 */
#[derive(Serialize,Deserialize,Debug)]
#[allow(non_snake_case)]
struct Entry {
    disasterNumber: i32,
    programTypeCode: String,
    programTypeDescription: String,
    stateCode: String,
    placeCode: String,
    placeName: String,
    designatedDate: DateTime<Utc>,
    entryDate: DateTime<Utc>,
    updateDate: DateTime<Utc>,
    hash: String,
    lastRefresh: DateTime<Utc>,
    id: String
}
#[derive(Deserialize,Debug)]
#[allow(non_snake_case)]
struct Response {
    FemaWebDeclarationAreas: Vec<Entry>,
}
#[derive(Deserialize,Debug)]
#[allow(non_snake_case)]
struct DeprecationInformation {
    depDate: DateTime<Utc>,
    deprecatedComment: String,
    depApiMessage: String,
    depNewURL: String,
    depWebMessage: String,
}
#[derive(Deserialize,Debug)]
#[allow(non_snake_case)]
struct Metadata {
    skip: i32,
    top: i32,
    count: i32,
    filter: String,
    format: String,
    metadata: bool,
    orderby: std::collections::HashMap<String,String>,
    select: String,
    entityname: String,
    version: String,
    url: String,
    rundate: DateTime<Utc>,
    DeprecationInformation: std::collections::HashMap<String,Option<String>>,
}
#[derive(Deserialize,Debug)]
#[allow(non_snake_case)]
struct ResponseWithMetaData {
    metadata: Metadata,
    FemaWebDeclarationAreas: Vec<Entry>,
}

// Helper function to make pagination less of a pain
fn get_uri(metadata: bool, base: &str, query: &str, page: usize, size: Option<usize>) -> String {
    let md_str = {
        if metadata {
            "on"
        }
        else {
            "off"
        }
    };
    match size {
        Some(s) => {
            format!("{}?{}&$skip={}&$top={}&$metadata={}", &base, &query, page*s, s, &md_str) 
        },
        _ => {
            format!("{}?{}&$metadata={}", &base, &query,&md_str)
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error+Send+Sync>> {
    let (cfg, log_cfg) = {
        if let Some(proj_dirs) = ProjectDirs::from("", "", &APPNAME) {
            let cfg_dir = proj_dirs.config_dir();
            let cfg: Config = confy::load(&APPNAME)?;
            let mut config_file = PathBuf::from(cfg_dir);
            config_file.push("log4rs");
            config_file.set_extension("yml");
            if !config_file.is_file() {
                std::fs::copy("log4rs.yml", &config_file)?;
            }
            let log_cfg = log4rs::load_config_file(&config_file, Default::default())?;
            (cfg, log_cfg)
        }
        else {
            panic!("Failed to load configuration files or safe defaults.");
        }
    };
    let _handle = log4rs::init_config(log_cfg);
    info!("Started logger.");
    let years_before = cfg.num_years_previous;
    let now: DateTime<Utc> = Utc::now();
    let cutoff = now - Duration::days(years_before as i64 * 365 as i64); 
    info!("Filtering for dates after {}.", cutoff);
    // Only request results with no closeoutDate key, filter a couple unneeded or redundant fields
    let base_uri = "https://www.fema.gov/api/open/v1/FemaWebDeclarationAreas";
    debug!("Base URI: {}", base_uri);
    let query = format!("$inlinecount=allpages&$select=disasterNumber,programTypeCode,programTypeDescription,stateCode,placeCode,placeName,designatedDate,entryDate,updateDate,hash,lastRefresh&$filter=designatedDate gt'{}' and closeoutDate eq null",
        cutoff.to_rfc3339_opts(chrono::SecondsFormat::Millis, true));
    let size: usize = 1000; // the maximum allowed and default for fema's API
    let response = reqwest::get(&get_uri(true, &base_uri, &query, 0, Some(size)))
                .await?
                .error_for_status()?
                .json::<ResponseWithMetaData>() // request metadata on the first run so that we can get the total count
                .await?;
    let count = response.metadata.count;
    info!("Server has {} matching results.", count);
    let mut entries = Vec::with_capacity(count as usize);
    for entry in response.FemaWebDeclarationAreas {
        entries.push(entry);
    }
    for page in 1 .. count as usize / size + 1 {
        let (start, mut end): (usize, i32) = (page*size, (page as i32+1)*size as i32);
        if end > count {
            end = count;
        }
        debug!("Requesting results {} through {}.", start, end);
        let response = reqwest::get(&get_uri(false, &base_uri, &query, page, Some(size)))
                .await?
                .error_for_status()?
                .json::<Response>() // Response will not contain the metadata
                .await?;
        debug!("Received results {} through {} from server.", start,end);
        for entry in response.FemaWebDeclarationAreas {
            entries.push(entry);
        }
    }
    info!("Number of results collected: {}", entries.len());
    if let Some(path) = &cfg.csv {
    let mut csvwriter = csv::Writer::from_path(path)?;
        for entry in entries {
            csvwriter.serialize(entry)?;
        }
        info!("Entries written to file {}.", path.to_str().unwrap());
    }
    Ok(())
}
