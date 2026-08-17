use mc_crypto::*;
fn hex(s:&str)->Vec<u8>{let mut v=Vec::new();let b=s.as_bytes();let mut i=0;while i+1<b.len(){v.push(((b[i] as char).to_digit(16).unwrap()*16+(b[i+1] as char).to_digit(16).unwrap()) as u8);i+=2;}v}
#[test]
fn fe_roundtrip(){
    let u:[u8;32]=hex("e6db6867583030db3594c1a424b15f7c726624ec26b3353b10a903a6d0ab1c4c").try_into().unwrap();
    let f=fe_frombytes_pub(&u);
    let b=fe_tobytes_pub(&f);
    assert_eq!(b, u, "roundtrip fail");
}
#[test]
fn fe_sq_one(){
    let one:[u64;5]=[1,0,0,0,0];
    let s=fe_mul_pub(&one,&one);
    assert_eq!(s, one, "1*1 != 1: {:?}", s);
}
#[test]
fn fe_mul_small(){
    let a:[u64;5]=[2,0,0,0,0];
    let b:[u64;5]=[3,0,0,0,0];
    let r=fe_mul_pub(&a,&b);
    assert_eq!(r, [6,0,0,0,0], "2*3: {:?}", r);
}
#[test]
fn x_debug_sc1(){
    let mut u2:[u8;32]=[0u8;32]; u2[0]=9;
    let mut sc=[0u8;32]; sc[0]=1;
    let (x2,z2,inv,out)=mc_crypto::x25519_raw_debug(&sc,&u2);
    let hx:Vec<String>=x2.iter().map(|b|format!("{:02x}",b)).collect();
    let hz:Vec<String>=z2.iter().map(|b|format!("{:02x}",b)).collect();
    let hi:Vec<String>=inv.iter().map(|b|format!("{:02x}",b)).collect();
    let ho:Vec<String>=out.iter().map(|b|format!("{:02x}",b)).collect();
    println!("x2={}",hx.join(""));
    println!("z2={}",hz.join(""));
    println!("inv={}",hi.join(""));
    println!("out={}",ho.join(""));
    println!("want out=0900000000000000000000000000000000000000000000000000000000000000");
}
#[test]
fn fe_sq_9(){
    let mut u2:[u8;32]=[0u8;32]; u2[0]=9;
    let f=fe_frombytes_pub(&u2);
    let s=fe_mul_pub(&f,&f);
    let b=fe_tobytes_pub(&s);
    // 81 mod p = 81
    let mut want=[0u8;32]; want[0]=81;
    assert_eq!(b, want, "9^2: {:?}", b);
}
#[test]
fn fe_mul_big(){
    // (p-1)^2 = 1?不,p-1 平方 = 1 mod p
    // p-1 = 2^255-20:低位字节 0xec,位 8..254 全 1 → [0xec, 0xff*30, 0x7f]
    let mut b=[0xffu8;32]; b[0]=0xec; b[31]=0x7f;
    let f=fe_frombytes_pub(&b);
    let s=fe_mul_pub(&f,&f);
    let o=fe_tobytes_pub(&s);
    let mut want=[0u8;32]; want[0]=1;
    assert_eq!(o, want, "(p-1)^2 should be 1: {:?}", o);
}
#[test]
fn fe_roundtrip_pm1(){
    let mut b=[0xffu8;32]; b[0]=0xec; b[31]=0x7f;
    let f=fe_frombytes_pub(&b);
    let o=fe_tobytes_pub(&f);
    assert_eq!(o, b, "roundtrip p-1: {:?}", o);
}
#[test]
fn fe_sq_pm1_limbs(){
    let mut b=[0xffu8;32]; b[0]=0xec;
    let f=fe_frombytes_pub(&b);
    let s=fe_mul_pub(&f,&f);
    println!("limbs={:?}", s);
    let o=fe_tobytes_pub(&s);
    println!("tobytes={:?}", o);
    let mut want=[0u8;32]; want[0]=1;
    assert_eq!(o, want, "(p-1)^2 should be 1: {:?}", o);
}
#[test]
fn fe_add_sub_rt(){
    let mut a:[u8;32]=[0u8;32]; a[0]=9;
    let mut b:[u8;32]=[0u8;32]; b[0]=5;
    let fa=fe_frombytes_pub(&a);
    let fb=fe_frombytes_pub(&b);
    // 没暴露 fe_add…… 跳过
    let _=(fa,fb);
}
#[test]
#[test]
fn ladder_trace(){
    let mut u2:[u8;32]=[0u8;32]; u2[0]=9;
    let mut sc=[0u8;32]; sc[0]=1;
    let mut x1=mc_crypto::fe_frombytes_pub(&u2);
    let mut x2=[1u64,0,0,0,0];
    let mut z2=[0u64,0,0,0,0];
    let mut x3=x1;
    let mut z3=[1u64,0,0,0,0];
    let mut swap=0u64;
    for t in (0..255).rev(){
        let k=(sc[t/8]>>(t%8))&1;
        swap^=k as u64;
        mc_crypto::fe_cswap_pub(&mut x2,&mut x3,swap);
        mc_crypto::fe_cswap_pub(&mut z2,&mut z3,swap);
        swap=k as u64;
        if t>=254||t==0{
            println!("IN  t={} k={} x2={:?} z2={:?} x3={:?} z3={:?}",t,k,x2,z2,x3,z3);
        }
        let a=mc_crypto::fe_add_pub(&x2,&z2);
        let b=mc_crypto::fe_sub_pub(&x2,&z2);
        let c=mc_crypto::fe_add_pub(&x3,&z3);
        let d=mc_crypto::fe_sub_pub(&x3,&z3);
        let da=mc_crypto::fe_mul_pub(&d,&a);
        let cb=mc_crypto::fe_mul_pub(&c,&b);
        let x3=mc_crypto::fe_add_pub(&da,&cb);
        let z3=mc_crypto::fe_sub_pub(&da,&cb);
        let x3=mc_crypto::fe_mul_pub(&x3,&x3);
        let z3=mc_crypto::fe_mul_pub(&z3,&z3);
        let z3=mc_crypto::fe_mul_pub(&z3,&x1);
        let aa=mc_crypto::fe_mul_pub(&a,&a);
        let bb=mc_crypto::fe_mul_pub(&b,&b);
        let e0=mc_crypto::fe_sub_pub(&aa,&bb);
        let e1=mc_crypto::fe_mul_pub(&e0,&[121665,0,0,0,0]);
        let e2=mc_crypto::fe_add_pub(&aa,&e1);
        let x2=mc_crypto::fe_mul_pub(&bb,&e2);
        let z2=mc_crypto::fe_mul_pub(&aa,&e0);
        if t>=254||t==0{
            println!("OUT t={} x2={:?} z2={:?}",t,x2,z2);
        }
    }
    mc_crypto::fe_cswap_pub(&mut x2,&mut x3,swap);
    mc_crypto::fe_cswap_pub(&mut z2,&mut z3,swap);
    println!("final x2={:?} z2={:?}",x2,z2);
}
#[test]
fn x_raw_sc1(){
    let mut u2:[u8;32]=[0u8;32]; u2[0]=9;
    let mut sc=[0u8;32]; sc[0]=1;
    let (x2,z2,inv,out)=mc_crypto::x25519_raw_debug(&sc,&u2);
    let hx:Vec<String>=x2.iter().map(|b|format!("{:02x}",b)).collect();
    let hz:Vec<String>=z2.iter().map(|b|format!("{:02x}",b)).collect();
    let hi:Vec<String>=inv.iter().map(|b|format!("{:02x}",b)).collect();
    let ho:Vec<String>=out.iter().map(|b|format!("{:02x}",b)).collect();
    println!("x2={}",hx.join(""));
    println!("z2={}",hz.join(""));
    println!("inv={}",hi.join(""));
    println!("out={}",ho.join(""));
    println!("want=0900000000000000000000000000000000000000000000000000000000000000");
}
#[test]
fn x_clamped_sc1(){
    let mut u2:[u8;32]=[0u8;32]; u2[0]=9;
    let mut sc=[0u8;32]; sc[0]=1;
    let out=mc_crypto::x25519_pub(&sc,&u2);
    let h:Vec<String>=out.iter().map(|b|format!("{:02x}",b)).collect();
    println!("out={}",h.join(""));
    println!("want=2fe57da347cd62431528daac5fbb290730fff684afc4cfc2ed90995f58cb3b74");
}
