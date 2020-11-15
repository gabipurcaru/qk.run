#![feature(proc_macro_hygiene, decl_macro)]

#[macro_use]
extern crate rocket;

#[macro_use]
extern crate lazy_static;

extern crate rusoto_core;
extern crate rusoto_dynamodb;
extern crate tokio;
extern crate yaml_rust;

use maplit::hashmap;
use regex::Regex;
use rocket::http::uri::Uri;
use rocket::response::{NamedFile, Redirect};
use rocket::Rocket;
use rocket_contrib::json::Json;
use rocket_contrib::serve::StaticFiles;
use rocket_contrib::templates::Template;
use rocket_lamb::RocketExt;
use rusoto_core::Region;
use rusoto_dynamodb::{AttributeValue, DynamoDb, DynamoDbClient, GetItemInput, PutItemInput};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::default::Default;
use tokio::runtime::Runtime;
use yaml_rust::yaml::Yaml;
use yaml_rust::YamlLoader;

const DEFAULT_YAML: &str = r#"# parameters you can use (see examples below):
# %q - the query, after the keyword
# %0, %1, %2 - the first, second or third parameter
# %hash - the hash of the search config

# This is an interactive editor - feel free to change and click save below!

# Default search engine - fallback to all queries
default: https://www.google.com/search?q=%q

# qk.run basics
help: { q: "https://qk.run/%hash", alias: ["edit", "list"] }

# Search engines
g: https://www.google.com/search?q=%q
d: https://duckduckgo.com/?q=%q

# Google
cal: https://calendar.google.com/calendar

# AWS
aws: https://console.aws.amazon.com/%q/home
s3: https://s3.console.aws.amazon.com/s3/buckets/%q
ec2: https://console.aws.amazon.com/ec2/%q
athena: https://console.aws.amazon.com/athena/home"#;

lazy_static! {
    static ref RUNTIME: Runtime = Runtime::new().unwrap();
}

lazy_static! {
    static ref DYNAMO_CLIENT: DynamoDbClient = DynamoDbClient::new(Region::EuWest2);
}

lazy_static! {
    static ref TWO_MATCH: Regex = Regex::new(r"(\S+)\s*(\S+)\s*(.*)").unwrap();
    static ref ONE_MATCH: Regex = Regex::new(r"(\S+)\s*(.*)").unwrap();
}

#[derive(Serialize)]
struct IndexTemplateContext {
    meta: HashMap<&'static str, &'static str>,
    yaml: String,
}

#[derive(Debug)]
struct RuleSet {
    default: Rule,
    rules: Vec<Rule>,
}

#[derive(Debug, Clone)]
struct Rule {
    prefix: String,
    query: String,
}

fn default_context(yaml: String) -> IndexTemplateContext {
    IndexTemplateContext {
        meta: hashmap! {
            "title" => "qk.run - search bar superpowers",
            "url" => "https://qk.run/"
        },
        yaml: yaml,
    }
}

fn render_page(yaml: String) -> Template {
    Template::render("index", &default_context(yaml))
}

fn read_yaml(hash_string: String) -> String {
    let res = DYNAMO_CLIENT
            .get_item(GetItemInput {
                table_name: "qkrun".to_string(),
                key: hashmap! {
                    "config_hash".to_string() => AttributeValue {s: Some(hash_string.clone()), ..Default::default()},
                },
                ..Default::default()
            });
    res.sync().unwrap().item.unwrap()["config"]
        .s
        .as_ref()
        .unwrap()
        .clone()
}

fn parse_yaml(yaml_string: String) -> RuleSet {
    let yaml = YamlLoader::load_from_str(&yaml_string).unwrap();

    let mut rules = RuleSet {
        default: Rule {
            prefix: "default".to_string(),
            query: "https://www.google.com/search?q=%q".to_string(),
        },
        rules: Vec::new(),
    };
    yaml[0].as_hash().unwrap().iter().for_each(|(k, v)| {
        let prefix = k.as_str().unwrap();

        match (v.as_str(), v.as_hash()) {
            (Some(query), _) => {
                rules.rules.push(Rule {
                    prefix: prefix.to_string(),
                    query: query.to_string(),
                });
            }
            (None, Some(h)) => {
                let query = &h[&Yaml::from_str("q")].as_str().unwrap();
                let alias = &h[&Yaml::from_str("alias")];

                let rule = Rule {
                    prefix: prefix.to_string(),
                    query: query.to_string(),
                };

                if prefix == "default" {
                    rules.default = rule.clone();
                    rules.default.prefix = "default".to_string();
                }
                rules.rules.push(rule);

                match (alias.as_str(), alias.as_vec()) {
                    (Some(alias_value), _) => {
                        let rule = Rule {
                            prefix: alias_value.to_string(),
                            query: query.to_string(),
                        };
                        if alias_value == "default" {
                            rules.default = rule.clone();
                            rules.default.prefix = "default".to_string();
                        }
                        rules.rules.push(rule);
                    }
                    (None, Some(alias_list)) => alias_list.iter().for_each(|alias| {
                        let rule = Rule {
                            prefix: alias.as_str().unwrap().to_string(),
                            query: query.to_string(),
                        };
                        if alias.as_str().unwrap() == "default" {
                            rules.default = rule.clone();
                            rules.default.prefix = "default".to_string();
                        }
                        rules.rules.push(rule);
                    }),
                    _ => {}
                }
            }
            _ => {}
        }
    });

    rules
}

#[get("/")]
fn index() -> Template {
    render_page(DEFAULT_YAML.to_string())
}

#[get("/<hash_string>")]
fn hash(hash_string: String) -> Template {
    render_page(read_yaml(hash_string))
}

#[get("/q/<hash_string>?<q>")]
fn query(hash_string: String, q: String) -> Redirect {
    let yaml_string = read_yaml(hash_string.clone());
    let rules = parse_yaml(yaml_string);

    let cap_two = TWO_MATCH.captures(&q);
    let zero = "";
    let one = "";

    // match &cap_two {
    //     Some(c) => {
    //         zero = &c[1];
    //         one = &c[2];
    //     }
    //     _ => {}
    // }

    let cap = ONE_MATCH.captures(&q);

    let mut word = "";
    let mut query = "";

    match &cap {
        Some(c) => {
            word = &c[1];
            query = &c[2];
        }
        _ => {}
    }

    // find matching rule
    let rule_matched = rules.rules.iter().find(|rule| rule.prefix == word);
    let rule = rule_matched.map_or(&rules.default, |r| &r);

    let mut q_replace = query;

    let encoded = Uri::percent_encode(&q).into_owned();
    let encoded_query = Uri::percent_encode(&query).into_owned();
    q_replace = &encoded;

    if rule.prefix != "default" {
        q_replace = &encoded_query;
    }

    // apply rule
    Redirect::to(
        rule.query
            .replace("%q", &q_replace)
            .replace("%hash", &hash_string)
            .replace("%0", &zero)
            .replace("%1", &one),
    )
}

#[get("/favicon.ico")]
fn favicon() -> Option<NamedFile> {
    NamedFile::open("assets/favicon.ico").ok()
}

#[derive(Deserialize)]
struct SaveInput {
    value: String,
}

#[post("/save", format = "application/json", data = "<yaml>")]
fn save(yaml: Json<SaveInput>) -> String {
    // validate the yaml
    YamlLoader::load_from_str(&yaml.value).unwrap();
    let client = DynamoDbClient::new(Region::EuWest2);
    let hash = format!("{:x}", md5::compute(&yaml.value));
    let res = client
            .put_item(PutItemInput {
                table_name: "qkrun".to_string(),
                condition_expression: Some("attribute_not_exists(config_hash)".to_string()),
                item: hashmap! {
                    "config_hash".to_string() => AttributeValue {s: Some(hash.clone()) , ..Default::default() },
                    "config".to_string() => AttributeValue { s: Some(yaml.value.clone()), ..Default::default()  },
                },
                ..Default::default()
            }).sync();
    hash
}

pub fn rocket() -> Rocket {
    rocket::ignite()
        .attach(Template::fairing())
        .mount("/assets", StaticFiles::from("assets"))
        // NOTE: the hash must be the last, since it's a catch all
        .mount("/", routes![favicon, save, index, query, hash])
}
