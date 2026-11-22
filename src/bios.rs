//! HP BIOS WMI mailbox (`root\wmi`, class `hpqBIntM`).
//!
//! Every call sends an `hpqBDataIn` { Sign = "SECU", Command, CommandType, Size, hpqBData }
//! to one of `hpqBIOSInt{0,4,128,1024,4096}` (the number is the reply size) and gets back
//! `OutData` { rwReturnCode, Data }.

use std::mem::ManuallyDrop;
use std::ptr::null_mut;

use windows::core::{w, Error, IUnknown, Interface, Result, BSTR, HSTRING, PCWSTR};
use windows::Win32::Foundation::E_FAIL;
use windows::Win32::System::Com::*;
use windows::Win32::System::Ole::*;
use windows::Win32::System::Variant::*;
use windows::Win32::System::Wmi::*;

pub struct Bios {
    svc: IWbemServices,
    path: BSTR,
    class: IWbemClassObject,
    data_in: IWbemClassObject,
}

pub fn connect(namespace: &str) -> Result<IWbemServices> {
    unsafe {
        let locator: IWbemLocator = CoCreateInstance(&WbemLocator, None, CLSCTX_INPROC_SERVER)?;
        let empty = BSTR::new();
        let svc = locator.ConnectServer(&BSTR::from(namespace), &empty, &empty, &empty, 0, &empty, None)?;
        // 10 = RPC_C_AUTHN_WINNT, 0 = RPC_C_AUTHZ_NONE
        CoSetProxyBlanket(&svc, 10, 0, PCWSTR::null(), RPC_C_AUTHN_LEVEL_CALL, RPC_C_IMP_LEVEL_IMPERSONATE, None, EOAC_NONE)?;
        Ok(svc)
    }
}

fn fail(msg: &str) -> Error {
    Error::new(E_FAIL, msg)
}

impl Bios {
    pub fn new() -> Result<Self> {
        unsafe {
            let svc = connect("root\\wmi")?;
            let class = get_class(&svc, "hpqBIntM")?;
            let data_in = get_class(&svc, "hpqBDataIn")?;

            let flags = WBEM_GENERIC_FLAG_TYPE(WBEM_FLAG_FORWARD_ONLY.0 | WBEM_FLAG_RETURN_IMMEDIATELY.0);
            let rows = svc.ExecQuery(&BSTR::from("WQL"), &BSTR::from("SELECT * FROM hpqBIntM"), flags, None)?;
            let mut objs = [None];
            let mut n = 0u32;
            rows.Next(-1, &mut objs, &mut n).ok()?;
            let inst = objs[0].take().ok_or_else(|| fail("hpqBIntM instance not found"))?;
            let mut v = VARIANT::default();
            inst.Get(w!("__PATH"), 0, &mut v, None, None)?;
            let inner: &VARIANT_0_0 = &v.Anonymous.Anonymous;
            if inner.vt != VT_BSTR {
                return Err(fail("hpqBIntM path missing"));
            }
            let path = (*inner.Anonymous.bstrVal).clone();
            Ok(Self { svc, path, class, data_in })
        }
    }

    /// Raw call. Returns BIOS return code and reply bytes.
    pub fn call(&self, cmd: u32, kind: u32, data: &[u8], out: usize) -> Result<(u32, Vec<u8>)> {
        let method = match out {
            0 => "hpqBIOSInt0",
            1..=4 => "hpqBIOSInt4",
            5..=128 => "hpqBIOSInt128",
            129..=1024 => "hpqBIOSInt1024",
            _ => "hpqBIOSInt4096",
        };
        unsafe {
            let mut in_sig = None;
            let mut out_sig = None;
            self.class.GetMethod(&HSTRING::from(method), 0, &mut in_sig, &mut out_sig)?;
            let params = in_sig.ok_or_else(|| fail("no in-signature"))?.SpawnInstance(0)?;

            let din = self.data_in.SpawnInstance(0)?;
            put(&din, w!("Sign"), bytes_variant(b"SECU"))?;
            put(&din, w!("Command"), VARIANT::from(cmd as i32))?;
            put(&din, w!("CommandType"), VARIANT::from(kind as i32))?;
            put(&din, w!("Size"), VARIANT::from(data.len() as i32))?;
            if !data.is_empty() {
                put(&din, w!("hpqBData"), bytes_variant(data))?;
            }
            put(&params, w!("InData"), VARIANT::from(din.cast::<IUnknown>()?))?;

            let mut result = None;
            self.svc.ExecMethod(&self.path, &BSTR::from(method), WBEM_GENERIC_FLAG_TYPE(0), None, &params, Some(&mut result), None)?;
            let result = result.ok_or_else(|| fail("no reply"))?;

            let mut v = VARIANT::default();
            result.Get(w!("OutData"), 0, &mut v, None, None)?;
            let reply: IWbemClassObject = IUnknown::try_from(&v)?.cast()?;

            let mut rc = VARIANT::default();
            reply.Get(w!("rwReturnCode"), 0, &mut rc, None, None)?;
            let rc = i32::try_from(&rc).unwrap_or(-1) as u32;

            let mut d = VARIANT::default();
            let bytes = if out > 0 && reply.Get(w!("Data"), 0, &mut d, None, None).is_ok() { variant_bytes(&d) } else { Vec::new() };
            Ok((rc, bytes))
        }
    }

    /// Read: errors on a non-zero BIOS return code.
    pub fn get(&self, cmd: u32, kind: u32, data: &[u8], out: usize) -> Result<Vec<u8>> {
        match self.call(cmd, kind, data, out)? {
            (0, bytes) => Ok(bytes),
            (rc, _) => Err(fail(&format!("BIOS 0x{cmd:X}/0x{kind:X} returned {rc}"))),
        }
    }

    /// Write: errors on a non-zero BIOS return code.
    pub fn set(&self, cmd: u32, kind: u32, data: &[u8]) -> Result<()> {
        self.get(cmd, kind, data, 4).map(|_| ())
    }
}

unsafe fn get_class(svc: &IWbemServices, name: &str) -> Result<IWbemClassObject> {
    let mut obj = None;
    svc.GetObject(&BSTR::from(name), WBEM_GENERIC_FLAG_TYPE(0), None, Some(&mut obj), None)?;
    obj.ok_or_else(|| fail("WMI class missing"))
}

unsafe fn put(obj: &IWbemClassObject, name: PCWSTR, v: VARIANT) -> Result<()> {
    obj.Put(name, 0, &v, 0)
}

fn bytes_variant(bytes: &[u8]) -> VARIANT {
    unsafe {
        let sa = SafeArrayCreateVector(VT_UI1, 0, bytes.len() as u32);
        let mut p = null_mut();
        if !sa.is_null() && SafeArrayAccessData(sa, &mut p).is_ok() {
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), p as *mut u8, bytes.len());
            let _ = SafeArrayUnaccessData(sa);
        }
        let mut v = VARIANT::default();
        let inner: &mut VARIANT_0_0 = &mut v.Anonymous.Anonymous;
        inner.vt = VARENUM(VT_ARRAY.0 | VT_UI1.0);
        inner.Anonymous.parray = sa;
        v
    }
}

fn variant_bytes(v: &VARIANT) -> Vec<u8> {
    unsafe {
        let inner: &ManuallyDrop<VARIANT_0_0> = &v.Anonymous.Anonymous;
        if inner.vt.0 & VT_ARRAY.0 == 0 {
            return Vec::new();
        }
        let sa = inner.Anonymous.parray;
        if sa.is_null() {
            return Vec::new();
        }
        let lo = SafeArrayGetLBound(sa, 1).unwrap_or(0);
        let hi = SafeArrayGetUBound(sa, 1).unwrap_or(-1);
        let n = (hi - lo + 1).max(0) as usize;
        let mut p = null_mut();
        if SafeArrayAccessData(sa, &mut p).is_err() {
            return Vec::new();
        }
        let out = std::slice::from_raw_parts(p as *const u8, n).to_vec();
        let _ = SafeArrayUnaccessData(sa);
        out
    }
}
