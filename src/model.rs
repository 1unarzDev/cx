use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Device { pub id:String, pub name:String, pub target:Option<String>, pub account:String, pub host:String, pub status:String, pub observed_at:u64 }
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Session { pub id:String, pub name:String, pub directory:String, pub provider:String, pub host:String, pub account:String, pub pid:u32, pub started:String, pub boot_id:String, pub external:bool, pub socket:Option<String> }
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CreateSession { pub key:String, pub directory:String, pub provider:String, pub name:String }
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag="op",content="args",rename_all="snake_case")]
pub enum Operation { Info, Sessions, Create(CreateSession), List{path:String}, Preview{path:String}, Mkdir{path:String}, Copy{source:String,destination:String,conflict:String,key:String}, Jobs, Cancel{key:String}, Network }
#[derive(Debug, Serialize, Deserialize)]
pub struct Request { pub version:u32, pub id:String, pub op:Operation }
#[derive(Debug, Serialize, Deserialize)]
pub struct Response { pub version:u32, pub id:String, pub result:Option<serde_json::Value>, pub error:Option<String> }
