use mc_crypto::x25519;
fn hex(s:&str)->Vec<u8>{let mut v=Vec::new();let b=s.as_bytes();let mut i=0;while i+1<b.len(){v.push(((b[i] as char).to_digit(16).unwrap()*16+(b[i+1] as char).to_digit(16).unwrap()) as u8);i+=2;}v}
fn main(){
    // scalar=1 → out 应 = u
    let mut u:[u8;32]=hex("e6db6867583030db3594c1a424b15f7c726624ec26b3353b10a903a6d0ab1c4c").try_into().unwrap();
    let mut sc=[0u8;32]; sc[0]=1;
    let out=x25519(&sc,&u);
    let mut s=String::new(); for b in out { s.push_str(&format!("{:02x}",b)); }
    println!("sc=1 out={}",s);
    // scalar=0 → out 应为 0
    sc=[0u8;32];
    let out=x25519(&sc,&u);
    let mut s=String::new(); for b in out { s.push_str(&format!("{:02x}",b)); }
    println!("sc=0 out={}",s);
    let _=&mut u;
}
