#![allow(unused_assignments)]

use anyhow::{anyhow, bail, Result};
use serde_json::json;
use spin_sdk::{
    http::{self, IntoResponse, Request, Response},
    http_component, redis,
};
use uuid::Uuid;

const REDIS_URL_ENV: &str = "REDIS_URL_ENV";
const CHANNEL_GATE2VIN: &str = "gate2vin";
const CHANNEL_VIN2WORKER: &str = "vin2worker";

/// A simple Spin HTTP component.
#[http_component]
fn http_gate(req: Request) -> Result<impl IntoResponse> {
    // println!("req: {:?}", req);

    let redis_addr = std::env::var(REDIS_URL_ENV)?;
    println!("redis_addr is: {}", redis_addr);
    let redis_conn = redis::Connection::open(&redis_addr)?;

    let path = req.path();
    let proto_name = parse_proto_name(&path);
    if &proto_name == "" {
        bail!("proto_name is empty.");
    }

    let mut method = String::new();
    let mut reqdata: Option<String> = None;
    match req.method() {
        http::Method::Get => {
            method = "query".to_owned();

            // In query mode: data is the url params
            let query_params = req.query();
            if query_params != "" {
                reqdata = Some(query_params.to_string());
            }
        }
        http::Method::Post => {
            method = "post".to_owned();

            // In post mode: data is the body content of the request
            let body = req.body();
            if body.is_empty() {
                reqdata = None;
            } else {
                let bo = String::from_utf8_lossy(body);
                reqdata = Some(bo.to_string());
            }
        }
        http::Method::Options => {
            return Ok(Response::builder()
                .status(200)
                .header("http_gate_version", "0.2")
                .header("Access-Control-Allow-Origin", "*")
                .header("Access-Control-Allow-Methods", "POST, GET, OPTIONS")
                .header("Access-Control-Allow-Headers", "X-PINGOTHER, Content-Type")
                .body("No data")
                .build());
        }
        _ => {
            // handle cases of other directives
            return Ok(Response::builder().status(500).body("No data").build());
        }
    };

    // We can do the unified authentication for some actions here
    // depends on path, method, and reqdata
    // XXX:

    // use a unique way to generate a reqid
    let reqid = Uuid::new_v4().simple().to_string();

    let payload = json!({
        "reqid": reqid,
        "reqdata": reqdata,
    });
    println!("payload: {:?}", payload);

    // construct a json, serialize it and send to a redis channel
    // model and action, we can plan a scheme to parse them out
    // here, we just put entire path content to action field, for later cases
    // we can parse it to model and action parts
    let json_to_send = json!({
        "proto": proto_name,
        "model": path,
        "action": &method,
        "data": payload.to_string().as_bytes().to_vec(),
        "ext": Vec::<u8>::new(),
    });

    if &method == "post" {
        // send to subxt proxy to handle
        _ = redis_conn.publish(
            CHANNEL_GATE2VIN,
            &serde_json::to_vec(&json_to_send).unwrap(),
        );
    } else if &method == "query" {
        let channel = format!("{}:{}", CHANNEL_VIN2WORKER, proto_name);
        // send to spin_redis_worker to handle
        _ = redis_conn.publish(&channel, &serde_json::to_vec(&json_to_send).unwrap());
    }

    let mut loop_count = 1;
    loop {
        let status_code = redis_conn
            .get(&format!("cache:status:{reqid}"))
            .map_err(|_| anyhow!("Error querying Redis"))?;

        if let Some(status_code) = status_code {
            // Now we get the raw serialized result from worker, we suppose it use
            // JSON spec to serialized it, so we can directly pass it back
            // to user's response body.
            let res_body = redis_conn
                .get(&format!("cache:{reqid}"))
                .map_err(|_| anyhow!("Error querying Redis"))?;
            let res_body = res_body.expect("empty response body.");
            // clear the redis cache key of the worker result
            let _ = redis_conn.del(&[format!("cache:status:{reqid}")]);
            let _ = redis_conn.del(&[format!("cache:{reqid}")]);

            let status_code = String::from_utf8(status_code).unwrap_or("500".to_string());
            let status_code = status_code.parse::<u16>().unwrap();
            // jump out this loop, and return the response to user
            return Ok(Response::builder()
                .status(status_code)
                .header("ef-http-gate-version", "1.0")
                .header("Access-Control-Allow-Origin", "*")
                .body(res_body)
                .build());
        } else {
            // after 20 seconds, timeout
            if loop_count < 4000 {
                // if not get the result, sleep for a little period
                let delta_millis = std::time::Duration::from_millis(5);
                std::thread::sleep(delta_millis);
                loop_count += 1;
            } else {
                println!("timeout, return 408");
                // timeout handler, use http status code
                return Ok(Response::builder()
                    .status(408)
                    .header("Access-Control-Allow-Origin", "*")
                    .body("Request Timeout")
                    .build());
            }
        }
    }
}

fn parse_proto_name(path: &str) -> String {
    path.trim_start_matches('/')
        .split('/')
        .next()
        .unwrap_or("")
        .to_string()
}
