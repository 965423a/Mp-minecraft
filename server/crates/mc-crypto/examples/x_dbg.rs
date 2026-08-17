use mc_crypto::x25519;
fn hex(s:&str)->Vec<u8>{let mut v=Vec::new();let b=s.as_bytes();let mut i=0;while i+1<b.len(){v.push(((b[i] as char).to_digit(16).unwrap()*16+(b[i+1] as char).to_digit(16).unwrap()) as u8);i+=2;}v}
fn main(){
    let scalar: [u8;32]=hex("a546e36bf0527c9d3b16154b82465edd62144c0ac1fc5a18506a2244ba449ac4").try_into().unwrap();
    let u: [u8;32]=hex("e6db6867583030db3594c1a424b15f7c726624ec26b3353b10a903a6d0ab1c4c").try_into().unwrap();
    let out=x25519(&scalar,&u);
    let mut s=String::new();
    for b in out { s.push_str(&format!("{:02x}",b)); }
    println!("out ={}",s);
    println!("want=c3da55379de9c6908e94ea4df28d084f32eccf03491c71f754b4075577a28552");
}
