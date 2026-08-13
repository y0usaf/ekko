fn main() {
    use ekko_proto::codec::{encode, decode};  // Wire trait in scope via encode bound
    let v: Vec<u8> = vec![1,2,3];
    let enc = encode(&v);
    println!("enc len={} bytes={:02x?}", enc.len(), enc);
    let dec: Vec<u8> = decode(&enc).unwrap();
    println!("dec={:?}", dec);
}
