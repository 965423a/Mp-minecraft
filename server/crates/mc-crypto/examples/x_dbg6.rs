use mc_crypto::x25519_raw;
fn main(){
    let mut u2:[u8;32]=[0u8;32]; u2[0]=9;
    let mut sc=[0u8;32]; sc[0]=1;
    let out=x25519_raw(&sc,&u2);
    let mut s=String::new(); for b in out { s.push_str(&format!("{:02x}",b)); }
    println!("sc=1(未clamp),u=9 out={}",s);
    println!("want=0900000000000000000000000000000000000000000000000000000000000000");
}
