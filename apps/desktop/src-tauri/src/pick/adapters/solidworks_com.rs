//! Native Rust Win32 COM (`IDispatch`) read-only probe for SolidWorks 2024–2026.
//!
//! Architecture:
//! - Runs strictly inside the medium-integrity Broker (or in-process when non-elevated) on a
//!   dedicated STA COM thread with a spec-mandated hard timeout (`SPEC_HARD_TIMEOUT_MS = 1500ms`).
//! - Zero `powershell.exe`, zero `Add-Type` / `csc.exe`, zero UIA calls on `SLDWORKS.exe`.
//! - Mode A (primary, strictly read-only): reads `ISelectionMgr` (up to `SPEC_MAX_TARGETS = 30`)
//!   without ever modifying or clearing the user's native selection set.
//! - Mode B (secondary, only when `sel_count == 0` & cursor is inside the 3D graphics viewport):
//!   computes Per-Monitor DPI-Aware physical client coordinates in Rust, derives the 3D ray from
//!   `IModelView.Transform` + `IModelView.GetVisibleBox()`, and probes `Extension.SelectByRay`
//!   using OS `SM_CXDRAG` tolerance.
//! - 3-Layer full coverage:
//!   1. Universal Envelope (all 149+ `swSelectType_e`, `GetPersistReference3` Base64, `component_chain`)
//!   2. 8 mutually-exclusive Family Enrichers (`topology`, `sketch_entity`, `feature_or_ref`,
//!      `body`, `dimension`, `assembly`, `drawing_annotation`, `generic_object`)
//!   3. O(1) `doc_context` (`GetWhatsWrongCount`/`GetWhatsWrong`, active sketch/component edit
//!      state, units, bodies, bounding box, custom properties) guarded by `SwAdaptiveProfiler`.

use crate::pick::adapters::solidworks::{
    classify_semantic_direction, invert_model_view_transform_to_ray, parse_equation_entry,
    sw_select_type_meta, SwAdaptiveProfiler, SwDocContext, SwDocInfo, SwEditState,
    SwModelSummary, SwPickMode, SwProbeOutcome, SwTargetFamily, SwTargetV2Item, SwTargetV2Payload,
    SwUnits, SPEC_BUSY_MAX_RETRIES, SPEC_BUSY_RETRY_DELAY_MS, SPEC_HARD_TIMEOUT_MS,
    SPEC_INTERACTIVE_TIMEOUT_MS, SPEC_MAX_CONFIG_NAMES, SPEC_MAX_CUSTOM_PROPERTIES,
    SPEC_MAX_ERROR_FEATURES, SPEC_MAX_PARENTS_CHILDREN, SPEC_MAX_TARGETS, SPEC_TARGET_SLA_MS,
    SW_SCHEMA_V2,
};
use base64::Engine;
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, HashMap};
use std::mem::ManuallyDrop;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use windows::core::{BSTR, GUID, Interface, IUnknown, PCWSTR};
use windows::Win32::Foundation::{
    HWND, LPARAM, POINT, RECT, VARIANT_BOOL, VARIANT_FALSE, VARIANT_TRUE, WPARAM,
};
use windows::Win32::Graphics::Gdi::{ClientToScreen, ScreenToClient};
use windows::Win32::System::Com::{
    CLSIDFromProgID, CoInitializeEx, CoUninitialize, IDispatch, COINIT_APARTMENTTHREADED,
    DISPATCH_FLAGS, DISPATCH_METHOD, DISPATCH_PROPERTYGET, DISPPARAMS, EXCEPINFO, SAFEARRAY,
};
use windows::Win32::System::Ole::{
    GetActiveObject, SafeArrayAccessData, SafeArrayGetLBound, SafeArrayGetUBound,
    SafeArrayGetVartype, SafeArrayUnaccessData,
};
use windows::Win32::System::Variant::{
    VariantClear, VARIANT, VARIANT_0, VARIANT_0_0, VARIANT_0_0_0, VARENUM, VT_ARRAY, VT_BOOL,
    VT_BSTR, VT_BYREF, VT_DISPATCH, VT_I2, VT_I4, VT_I8, VT_R4, VT_R8, VT_UI1,
    VT_UI2, VT_UI4, VT_UI8, VT_UNKNOWN, VT_VARIANT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetClientRect, GetSystemMetrics, GetWindowRect, PostMessageW, WindowFromPoint, SM_CXDRAG,
    WM_LBUTTONDOWN, WM_LBUTTONUP,
};

const RPC_E_CALL_REJECTED_CODE: i32 = 0x8001_0001u32 as i32;
const RPC_E_SERVERCALL_RETRYLATER_CODE: i32 = 0x8001_010Au32 as i32;

// SolidWorks official `swUserPreferenceIntegerValue_e` constants
const SW_UNITS_LINEAR: i32 = 24;
const SW_UNITS_ANGULAR: i32 = 26;

static ADAPTIVE_PROFILER: OnceLock<Mutex<SwAdaptiveProfiler>> = OnceLock::new();

fn adaptive_profiler() -> &'static Mutex<SwAdaptiveProfiler> {
    ADAPTIVE_PROFILER.get_or_init(|| Mutex::new(SwAdaptiveProfiler::new(SPEC_TARGET_SLA_MS)))
}

use crate::win32_util::to_wide as to_wide_null;

fn round_dec(v: f64, places: i32) -> f64 {
    if !v.is_finite() {
        return 0.0;
    }
    let f = 10f64.powi(places);
    (v * f).round() / f
}

fn is_com_busy_hresult(code: i32) -> bool {
    code == RPC_E_CALL_REJECTED_CODE || code == RPC_E_SERVERCALL_RETRYLATER_CODE
}

/// RAII wrapper ensuring Win32 `VARIANT` (including `BSTR`, `IDispatch`, `SAFEARRAY`) is always
/// cleared on drop.
struct VariantGuard(VARIANT);

impl VariantGuard {
    fn empty() -> Self {
        Self(VARIANT::default())
    }

    fn from_i32(v: i32) -> Self {
        Self(VARIANT {
            Anonymous: VARIANT_0 {
                Anonymous: ManuallyDrop::new(VARIANT_0_0 {
                    vt: VT_I4,
                    wReserved1: 0,
                    wReserved2: 0,
                    wReserved3: 0,
                    Anonymous: VARIANT_0_0_0 { lVal: v },
                }),
            },
        })
    }

    fn from_f64(v: f64) -> Self {
        Self(VARIANT {
            Anonymous: VARIANT_0 {
                Anonymous: ManuallyDrop::new(VARIANT_0_0 {
                    vt: VT_R8,
                    wReserved1: 0,
                    wReserved2: 0,
                    wReserved3: 0,
                    Anonymous: VARIANT_0_0_0 { dblVal: v },
                }),
            },
        })
    }

    fn from_bool(v: bool) -> Self {
        Self(VARIANT {
            Anonymous: VARIANT_0 {
                Anonymous: ManuallyDrop::new(VARIANT_0_0 {
                    vt: VT_BOOL,
                    wReserved1: 0,
                    wReserved2: 0,
                    wReserved3: 0,
                    Anonymous: VARIANT_0_0_0 {
                        boolVal: if v { VARIANT_TRUE } else { VARIANT_FALSE },
                    },
                }),
            },
        })
    }

    fn from_str(s: &str) -> Self {
        let bstr = BSTR::from(s);
        Self(VARIANT {
            Anonymous: VARIANT_0 {
                Anonymous: ManuallyDrop::new(VARIANT_0_0 {
                    vt: VT_BSTR,
                    wReserved1: 0,
                    wReserved2: 0,
                    wReserved3: 0,
                    Anonymous: VARIANT_0_0_0 {
                        bstrVal: ManuallyDrop::new(bstr),
                    },
                }),
            },
        })
    }

    fn from_dispatch(disp: &IDispatch) -> Self {
        Self(VARIANT {
            Anonymous: VARIANT_0 {
                Anonymous: ManuallyDrop::new(VARIANT_0_0 {
                    vt: VT_DISPATCH,
                    wReserved1: 0,
                    wReserved2: 0,
                    wReserved3: 0,
                    Anonymous: VARIANT_0_0_0 {
                        pdispVal: ManuallyDrop::new(Some(disp.clone())),
                    },
                }),
            },
        })
    }

    /// Creates a `VT_VARIANT | VT_BYREF` pointer pointing to a caller-owned `VARIANT` slot
    /// (used for COM `[out]` parameters such as `GetWhatsWrong` and `GetAll3`).
    /// Note: `VariantClear` is a no-op on `VT_BYREF`, so dropping this guard never frees the target.
    fn byref_variant(slot: &mut VARIANT) -> Self {
        Self(VARIANT {
            Anonymous: VARIANT_0 {
                Anonymous: ManuallyDrop::new(VARIANT_0_0 {
                    vt: VARENUM(VT_VARIANT.0 | VT_BYREF.0),
                    wReserved1: 0,
                    wReserved2: 0,
                    wReserved3: 0,
                    Anonymous: VARIANT_0_0_0 {
                        pvarVal: slot as *mut VARIANT,
                    },
                }),
            },
        })
    }

    fn byref_bool(slot: &mut VARIANT_BOOL) -> Self {
        Self(VARIANT {
            Anonymous: VARIANT_0 {
                Anonymous: ManuallyDrop::new(VARIANT_0_0 {
                    vt: VARENUM(VT_BOOL.0 | VT_BYREF.0),
                    wReserved1: 0,
                    wReserved2: 0,
                    wReserved3: 0,
                    Anonymous: VARIANT_0_0_0 {
                        pboolVal: slot as *mut VARIANT_BOOL,
                    },
                }),
            },
        })
    }

    fn as_string(&self) -> Option<String> {
        unsafe {
            let v = &*self.0.Anonymous.Anonymous;
            match v.vt {
                x if x == VT_BSTR => Some(v.Anonymous.bstrVal.to_string()),
                x if x.0 == (VT_BSTR.0 | VT_BYREF.0) && !v.Anonymous.pbstrVal.is_null() => {
                    Some((*v.Anonymous.pbstrVal).to_string())
                }
                x if x == VT_I4 => Some(v.Anonymous.lVal.to_string()),
                x if x == VT_I2 => Some(v.Anonymous.iVal.to_string()),
                x if x == VT_R8 => Some(v.Anonymous.dblVal.to_string()),
                x if x == VT_BOOL => Some((v.Anonymous.boolVal.0 != 0).to_string()),
                _ => None,
            }
        }
    }

    fn as_i32(&self) -> Option<i32> {
        unsafe {
            let v = &*self.0.Anonymous.Anonymous;
            match v.vt {
                x if x == VT_I4 => Some(v.Anonymous.lVal),
                x if x == VT_I2 => Some(v.Anonymous.iVal as i32),
                x if x == VT_I8 => Some(v.Anonymous.llVal as i32),
                x if x == VT_UI1 => Some(v.Anonymous.bVal as i32),
                x if x == VT_UI2 => Some(v.Anonymous.uiVal as i32),
                x if x == VT_UI4 => Some(v.Anonymous.ulVal as i32),
                x if x == VT_R8 => Some(v.Anonymous.dblVal as i32),
                x if x == VT_BOOL => Some(if v.Anonymous.boolVal.0 != 0 { 1 } else { 0 }),
                _ => None,
            }
        }
    }

    fn as_i64(&self) -> Option<i64> {
        unsafe {
            let v = &*self.0.Anonymous.Anonymous;
            match v.vt {
                x if x == VT_I8 => Some(v.Anonymous.llVal),
                x if x == VT_UI8 => Some(v.Anonymous.ullVal as i64),
                x if x == VT_I4 => Some((v.Anonymous.lVal as u32) as i64),
                x if x == VT_UI4 => Some(v.Anonymous.ulVal as i64),
                x if x == VT_I2 => Some(v.Anonymous.iVal as i64),
                x if x == VT_R8 => Some(v.Anonymous.dblVal as i64),
                _ => None,
            }
        }
    }

    fn as_f64(&self) -> Option<f64> {
        unsafe {
            let v = &*self.0.Anonymous.Anonymous;
            match v.vt {
                x if x == VT_R8 => Some(v.Anonymous.dblVal),
                x if x == VT_R4 => Some(v.Anonymous.fltVal as f64),
                x if x == VT_I4 => Some(v.Anonymous.lVal as f64),
                x if x == VT_I2 => Some(v.Anonymous.iVal as f64),
                x if x == VT_I8 => Some(v.Anonymous.llVal as f64),
                _ => None,
            }
        }
    }

    fn as_bool(&self) -> Option<bool> {
        unsafe {
            let v = &*self.0.Anonymous.Anonymous;
            match v.vt {
                x if x == VT_BOOL => Some(v.Anonymous.boolVal.0 != 0),
                x if x == VT_I4 => Some(v.Anonymous.lVal != 0),
                x if x == VT_I2 => Some(v.Anonymous.iVal != 0),
                _ => None,
            }
        }
    }

    fn as_dispatch(&self) -> Option<DispObj> {
        unsafe {
            let v = &*self.0.Anonymous.Anonymous;
            if v.vt == VT_DISPATCH {
                let d: Option<IDispatch> = (*v.Anonymous.pdispVal).clone();
                d.map(DispObj)
            } else if v.vt == VT_UNKNOWN {
                let unk: Option<IUnknown> = (*v.Anonymous.punkVal).clone();
                unk.and_then(|u| u.cast::<IDispatch>().ok()).map(DispObj)
            } else {
                None
            }
        }
    }

    fn safearray_ptr(&self) -> Option<*mut SAFEARRAY> {
        unsafe {
            let v = &*self.0.Anonymous.Anonymous;
            if (v.vt.0 & VT_ARRAY.0) == 0 {
                return None;
            }
            let psa = if (v.vt.0 & VT_BYREF.0) != 0 {
                if v.Anonymous.pparray.is_null() {
                    return None;
                }
                *v.Anonymous.pparray
            } else {
                v.Anonymous.parray
            };
            if psa.is_null() {
                None
            } else {
                Some(psa)
            }
        }
    }

    fn as_bytes(&self) -> Option<Vec<u8>> {
        let psa = self.safearray_ptr()?;
        unsafe {
            let lbound = SafeArrayGetLBound(psa, 1).ok()?;
            let ubound = SafeArrayGetUBound(psa, 1).ok()?;
            if ubound < lbound {
                return Some(Vec::new());
            }
            let len = (ubound - lbound + 1) as usize;
            let elem_vt = SafeArrayGetVartype(psa).ok()?;
            let mut data: *mut std::ffi::c_void = std::ptr::null_mut();
            SafeArrayAccessData(psa, &mut data).ok()?;
            let out = if elem_vt == VT_UI1 {
                let slice = std::slice::from_raw_parts(data as *const u8, len);
                Some(slice.to_vec())
            } else if elem_vt == VT_VARIANT {
                let slice = std::slice::from_raw_parts(data as *const VARIANT, len);
                let mut bytes = Vec::with_capacity(len);
                for item in slice {
                    let iv = &*item.Anonymous.Anonymous;
                    if iv.vt == VT_UI1 {
                        bytes.push(iv.Anonymous.bVal);
                    } else if iv.vt == VT_I4 {
                        bytes.push((iv.Anonymous.lVal & 0xFF) as u8);
                    } else if iv.vt == VT_I2 {
                        bytes.push((iv.Anonymous.iVal & 0xFF) as u8);
                    }
                }
                Some(bytes)
            } else {
                None
            };
            let _ = SafeArrayUnaccessData(psa);
            out
        }
    }

    fn as_f64_vec(&self) -> Option<Vec<f64>> {
        let psa = self.safearray_ptr()?;
        unsafe {
            let lbound = SafeArrayGetLBound(psa, 1).ok()?;
            let ubound = SafeArrayGetUBound(psa, 1).ok()?;
            if ubound < lbound {
                return Some(Vec::new());
            }
            let len = (ubound - lbound + 1) as usize;
            let elem_vt = SafeArrayGetVartype(psa).ok()?;
            let mut data: *mut std::ffi::c_void = std::ptr::null_mut();
            SafeArrayAccessData(psa, &mut data).ok()?;
            let out = if elem_vt == VT_R8 {
                let slice = std::slice::from_raw_parts(data as *const f64, len);
                Some(slice.to_vec())
            } else if elem_vt == VT_R4 {
                let slice = std::slice::from_raw_parts(data as *const f32, len);
                Some(slice.iter().map(|&x| x as f64).collect())
            } else if elem_vt == VT_VARIANT {
                let slice = std::slice::from_raw_parts(data as *const VARIANT, len);
                let mut vals = Vec::with_capacity(len);
                for item in slice {
                    let iv = &*item.Anonymous.Anonymous;
                    let v = match iv.vt {
                        x if x == VT_R8 => iv.Anonymous.dblVal,
                        x if x == VT_R4 => iv.Anonymous.fltVal as f64,
                        x if x == VT_I4 => iv.Anonymous.lVal as f64,
                        x if x == VT_I2 => iv.Anonymous.iVal as f64,
                        _ => continue,
                    };
                    vals.push(v);
                }
                Some(vals)
            } else {
                None
            };
            let _ = SafeArrayUnaccessData(psa);
            out
        }
    }

    fn as_bool_vec(&self) -> Option<Vec<bool>> {
        let psa = self.safearray_ptr()?;
        unsafe {
            let lbound = SafeArrayGetLBound(psa, 1).ok()?;
            let ubound = SafeArrayGetUBound(psa, 1).ok()?;
            if ubound < lbound {
                return Some(Vec::new());
            }
            let len = (ubound - lbound + 1) as usize;
            let elem_vt = SafeArrayGetVartype(psa).ok()?;
            let mut data: *mut std::ffi::c_void = std::ptr::null_mut();
            SafeArrayAccessData(psa, &mut data).ok()?;
            let out = if elem_vt == VT_BOOL {
                let slice = std::slice::from_raw_parts(data as *const VARIANT_BOOL, len);
                Some(slice.iter().map(|b| b.0 != 0).collect())
            } else if elem_vt == VT_VARIANT {
                let slice = std::slice::from_raw_parts(data as *const VARIANT, len);
                let mut vals = Vec::with_capacity(len);
                for item in slice {
                    let iv = &*item.Anonymous.Anonymous;
                    let b = match iv.vt {
                        x if x == VT_BOOL => iv.Anonymous.boolVal.0 != 0,
                        x if x == VT_I4 => iv.Anonymous.lVal != 0,
                        _ => false,
                    };
                    vals.push(b);
                }
                Some(vals)
            } else {
                None
            };
            let _ = SafeArrayUnaccessData(psa);
            out
        }
    }

    fn as_string_vec(&self) -> Option<Vec<String>> {
        let psa = self.safearray_ptr()?;
        unsafe {
            let lbound = SafeArrayGetLBound(psa, 1).ok()?;
            let ubound = SafeArrayGetUBound(psa, 1).ok()?;
            if ubound < lbound {
                return Some(Vec::new());
            }
            let len = (ubound - lbound + 1) as usize;
            let elem_vt = SafeArrayGetVartype(psa).ok()?;
            let mut data: *mut std::ffi::c_void = std::ptr::null_mut();
            SafeArrayAccessData(psa, &mut data).ok()?;
            let out = if elem_vt == VT_BSTR {
                let slice = std::slice::from_raw_parts(data as *const BSTR, len);
                Some(slice.iter().map(|b| b.to_string()).collect())
            } else if elem_vt == VT_VARIANT {
                let slice = std::slice::from_raw_parts(data as *const VARIANT, len);
                let mut vals = Vec::with_capacity(len);
                for item in slice {
                    let iv = &*item.Anonymous.Anonymous;
                    if iv.vt == VT_BSTR {
                        vals.push(iv.Anonymous.bstrVal.to_string());
                    } else if iv.vt == VT_I4 {
                        vals.push(iv.Anonymous.lVal.to_string());
                    } else if iv.vt == VT_R8 {
                        vals.push(iv.Anonymous.dblVal.to_string());
                    }
                }
                Some(vals)
            } else {
                None
            };
            let _ = SafeArrayUnaccessData(psa);
            out
        }
    }

    fn as_dispatch_vec(&self) -> Option<Vec<DispObj>> {
        let psa = self.safearray_ptr()?;
        unsafe {
            let lbound = SafeArrayGetLBound(psa, 1).ok()?;
            let ubound = SafeArrayGetUBound(psa, 1).ok()?;
            if ubound < lbound {
                return Some(Vec::new());
            }
            let len = (ubound - lbound + 1) as usize;
            let elem_vt = SafeArrayGetVartype(psa).ok()?;
            let mut data: *mut std::ffi::c_void = std::ptr::null_mut();
            SafeArrayAccessData(psa, &mut data).ok()?;
            let out = if elem_vt == VT_DISPATCH {
                let slice = std::slice::from_raw_parts(data as *const Option<IDispatch>, len);
                Some(slice.iter().filter_map(|d| d.clone().map(DispObj)).collect())
            } else if elem_vt == VT_UNKNOWN {
                let slice = std::slice::from_raw_parts(data as *const Option<IUnknown>, len);
                Some(
                    slice
                        .iter()
                        .filter_map(|u| u.as_ref().and_then(|x| x.cast::<IDispatch>().ok()).map(DispObj))
                        .collect(),
                )
            } else if elem_vt == VT_VARIANT {
                let slice = std::slice::from_raw_parts(data as *const VARIANT, len);
                let mut vals = Vec::with_capacity(len);
                for item in slice {
                    let iv = &*item.Anonymous.Anonymous;
                    if iv.vt == VT_DISPATCH {
                        if let Some(d) = (*iv.Anonymous.pdispVal).clone() {
                            vals.push(DispObj(d));
                        }
                    } else if iv.vt == VT_UNKNOWN {
                        if let Some(u) = (*iv.Anonymous.punkVal).clone() {
                            if let Ok(d) = u.cast::<IDispatch>() {
                                vals.push(DispObj(d));
                            }
                        }
                    }
                }
                Some(vals)
            } else {
                None
            };
            let _ = SafeArrayUnaccessData(psa);
            out
        }
    }
}

impl Drop for VariantGuard {
    fn drop(&mut self) {
        unsafe {
            let _ = VariantClear(&mut self.0);
        }
    }
}

#[derive(Clone)]
struct DispObj(IDispatch);

enum ComCallErr {
    Busy,
    Failed,
}

impl DispObj {
    fn disp_id(&self, name: &str) -> Result<i32, ComCallErr> {
        let wide = to_wide_null(name);
        let names = [PCWSTR(wide.as_ptr())];
        let mut id = 0i32;
        unsafe {
            match self
                .0
                .GetIDsOfNames(&GUID::zeroed(), names.as_ptr(), 1, 0x0409, &mut id)
            {
                Ok(()) => Ok(id),
                Err(e) => {
                    if is_com_busy_hresult(e.code().0) {
                        Err(ComCallErr::Busy)
                    } else {
                        Err(ComCallErr::Failed)
                    }
                }
            }
        }
    }

    fn invoke_raw(
        &self,
        name: &str,
        flags: DISPATCH_FLAGS,
        mut args_in_natural_order: Vec<VariantGuard>,
    ) -> Result<VariantGuard, ComCallErr> {
        let id = self.disp_id(name)?;
        // COM IDispatch::Invoke requires arguments in REVERSE order (last arg at index 0).
        // Use `ManuallyDrop<VARIANT>` so the `Vec` heap allocation frees normally without
        // double-dropping the shallow-copied `VARIANT`s owned by `args_in_natural_order`!
        args_in_natural_order.reverse();
        let mut raw_args: Vec<std::mem::ManuallyDrop<VARIANT>> = args_in_natural_order
            .iter()
            .map(|g| std::mem::ManuallyDrop::new(unsafe { std::ptr::read(&g.0) }))
            .collect();
        let params = DISPPARAMS {
            rgvarg: if raw_args.is_empty() {
                std::ptr::null_mut()
            } else {
                raw_args.as_mut_ptr() as *mut VARIANT
            },
            rgdispidNamedArgs: std::ptr::null_mut(),
            cArgs: raw_args.len() as u32,
            cNamedArgs: 0,
        };
        let mut result = VariantGuard::empty();
        let mut excep = EXCEPINFO::default();
        let mut arg_err = 0u32;
        let hr = unsafe {
            self.0.Invoke(
                id,
                &GUID::zeroed(),
                0x0409,
                flags,
                &params,
                Some(&mut result.0),
                Some(&mut excep),
                Some(&mut arg_err),
            )
        };
        drop(raw_args);
        match hr {
            Ok(()) => Ok(result),
            Err(e) => {
                if is_com_busy_hresult(e.code().0) {
                    Err(ComCallErr::Busy)
                } else {
                    Err(ComCallErr::Failed)
                }
            }
        }
    }

    fn get(&self, prop: &str) -> Result<VariantGuard, ComCallErr> {
        self.invoke_raw(prop, DISPATCH_PROPERTYGET | DISPATCH_METHOD, Vec::new())
    }

    fn get_with(&self, prop: &str, args: Vec<VariantGuard>) -> Result<VariantGuard, ComCallErr> {
        self.invoke_raw(prop, DISPATCH_PROPERTYGET | DISPATCH_METHOD, args)
    }

    fn call(&self, method: &str, args: Vec<VariantGuard>) -> Result<VariantGuard, ComCallErr> {
        self.invoke_raw(method, DISPATCH_METHOD, args)
    }

    fn get_str(&self, prop: &str) -> String {
        self.get(prop)
            .ok()
            .and_then(|v| v.as_string())
            .unwrap_or_default()
    }

    fn get_i32(&self, prop: &str) -> Option<i32> {
        self.get(prop).ok().and_then(|v| v.as_i32())
    }

    fn get_f64(&self, prop: &str) -> Option<f64> {
        self.get(prop).ok().and_then(|v| v.as_f64())
    }

    fn get_bool(&self, prop: &str) -> Option<bool> {
        self.get(prop).ok().and_then(|v| v.as_bool())
    }

    fn get_disp(&self, prop: &str) -> Option<DispObj> {
        self.get(prop).ok().and_then(|v| v.as_dispatch())
    }

    fn com_type_name(&self) -> Option<String> {
        unsafe {
            let ti = self.0.GetTypeInfo(0, 0x0409).ok()?;
            let mut bstr_name = BSTR::new();
            ti.GetDocumentation(-1, Some(&mut bstr_name), None, std::ptr::null_mut(), None)
                .ok()?;
            let s = bstr_name.to_string();
            if s.trim().is_empty() {
                None
            } else {
                Some(s)
            }
        }
    }
}

fn connect_active_solidworks() -> Result<DispObj, ComCallErr> {
    let prog_id = to_wide_null("SldWorks.Application");
    let clsid = unsafe { CLSIDFromProgID(PCWSTR(prog_id.as_ptr())) }.map_err(|_| ComCallErr::Failed)?;
    let mut unk: Option<IUnknown> = None;
    unsafe {
        if let Err(e) = GetActiveObject(&clsid, None, &mut unk) {
            if is_com_busy_hresult(e.code().0) {
                return Err(ComCallErr::Busy);
            }
            return Err(ComCallErr::Failed);
        }
    }
    let unk = unk.ok_or(ComCallErr::Failed)?;
    let disp: IDispatch = unk.cast().map_err(|_| ComCallErr::Failed)?;
    Ok(DispObj(disp))
}

/// Serializes active SolidWorks COM STA threads so a slow/timed-out probe can NEVER overlap with
/// a newly spawned probe hammering SolidWorks's single-threaded STA message pump concurrently.
static SW_COM_INFLIGHT: AtomicBool = AtomicBool::new(false);

struct SwComInflightGuard;
impl Drop for SwComInflightGuard {
    fn drop(&mut self) {
        SW_COM_INFLIGHT.store(false, Ordering::Release);
    }
}

/// Runs the native SolidWorks COM probe on a dedicated STA thread with strict hard timeout
/// (`SPEC_HARD_TIMEOUT_MS = 1500ms` for read-only Mode A; `SPEC_INTERACTIVE_TIMEOUT_MS = 3500ms`
/// for interactive ray/tree picks) and FR-4 `RPC_E_CALL_REJECTED` 1-retry (150ms) guard.
pub fn probe_solidworks_in_medium(
    screen_pos: Option<(i32, i32)>,
    pick_mode: SwPickMode,
    timeout_ms: u64,
) -> SwProbeOutcome {
    let max_cap = match pick_mode {
        SwPickMode::CurrentSelectionOnly => SPEC_HARD_TIMEOUT_MS,
        SwPickMode::ViewportRayOnly | SwPickMode::FeatureTreeClick | SwPickMode::Auto => {
            SPEC_INTERACTIVE_TIMEOUT_MS
        }
    };
    let effective_timeout = if timeout_ms == 0 {
        max_cap
    } else {
        timeout_ms.min(max_cap)
    };

    // If a prior timed-out STA probe is still unwinding inside SolidWorks COM, wait briefly for it
    // to finish rather than launching a second competing STA thread onto the same COM apartment.
    let drain_deadline =
        Instant::now() + Duration::from_millis((effective_timeout / 4).clamp(150, 450));
    while SW_COM_INFLIGHT
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        if Instant::now() >= drain_deadline {
            log::warn!(
                "SolidWorks native COM probe skipped: prior STA probe still in flight; reporting Busy"
            );
            return SwProbeOutcome::Busy;
        }
        std::thread::sleep(Duration::from_millis(20));
    }

    let (tx, rx) = mpsc::channel::<SwProbeOutcome>();

    let spawn_res = std::thread::Builder::new()
        .name("atp-sw-com-sta".to_string())
        .spawn(move || {
            let _guard = SwComInflightGuard;
            let outcome = run_sta_probe_with_retry(screen_pos, pick_mode, effective_timeout);
            let _ = tx.send(outcome);
        });

    if spawn_res.is_err() {
        SW_COM_INFLIGHT.store(false, Ordering::Release);
        return SwProbeOutcome::Unavailable;
    }

    match rx.recv_timeout(Duration::from_millis(effective_timeout)) {
        Ok(outcome) => outcome,
        Err(_) => {
            log::warn!(
                "SolidWorks native COM probe exceeded hard timeout ({effective_timeout}ms); degrading safely"
            );
            SwProbeOutcome::TimedOut
        }
    }
}

fn run_sta_probe_with_retry(
    screen_pos: Option<(i32, i32)>,
    pick_mode: SwPickMode,
    timeout_ms: u64,
) -> SwProbeOutcome {
    let started = Instant::now();
    let hard_deadline = started + Duration::from_millis(timeout_ms);

    let com_inited = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED).is_ok() };

    let mut attempt = 0u32;
    let outcome = loop {
        match run_single_probe_attempt(screen_pos, pick_mode, started, hard_deadline) {
            Ok(Some(payload)) => break SwProbeOutcome::Ok(payload),
            Ok(None) => break SwProbeOutcome::Unavailable,
            Err(ComCallErr::Busy) => {
                if attempt < SPEC_BUSY_MAX_RETRIES
                    && Instant::now() + Duration::from_millis(SPEC_BUSY_RETRY_DELAY_MS) < hard_deadline
                {
                    attempt += 1;
                    std::thread::sleep(Duration::from_millis(SPEC_BUSY_RETRY_DELAY_MS));
                    continue;
                }
                break SwProbeOutcome::Busy;
            }
            Err(ComCallErr::Failed) => break SwProbeOutcome::Unavailable,
        }
    };

    if com_inited {
        unsafe {
            CoUninitialize();
        }
    }
    outcome
}

fn run_single_probe_attempt(
    screen_pos: Option<(i32, i32)>,
    requested_mode: SwPickMode,
    started: Instant,
    hard_deadline: Instant,
) -> Result<Option<SwTargetV2Payload>, ComCallErr> {
    let sw = connect_active_solidworks()?;
    let doc_var = sw.get("ActiveDoc")?;
    let Some(doc) = doc_var.as_dispatch() else {
        return Ok(None);
    };

    let doc_title = doc.get_str("GetTitle");
    let doc_path = doc.get_str("GetPathName");
    let doc_type_num = doc.get_i32("GetType").unwrap_or(0);
    let doc_type = match doc_type_num {
        1 => "Part",
        2 => "Assembly",
        3 => "Drawing",
        _ => "Unknown",
    }
    .to_string();

    let doc_key = if !doc_path.is_empty() {
        doc_path.clone()
    } else {
        doc_title.clone()
    };

    let skip_flags = adaptive_profiler()
        .lock()
        .ok()
        .map(|mut p| p.begin_doc(&doc_key))
        .unwrap_or_default();

    let sm_var = doc.get("SelectionManager")?;
    let sm = sm_var.as_dispatch().ok_or(ComCallErr::Failed)?;
    let ext = doc.get_disp("Extension");

    let mut sel_start = 1i32;
    let mut sel_end = 0i32;
    let mut pick_mode = String::new();

    match requested_mode {
        SwPickMode::CurrentSelectionOnly => {
            sel_end = sm
                .call("GetSelectedObjectCount2", vec![VariantGuard::from_i32(-1)])?
                .as_i32()
                .unwrap_or(0);
            if sel_end > 0 {
                pick_mode = "current_selection".to_string();
            }
        }
        SwPickMode::ViewportRayOnly => {
            // Explicitly ray-pick the entity under cursor (px, py) without returning stale pre-selection.
            // Because Ctrl is physically held during Pen mode, SolidWorks `SelectByRay` may append to
            // an existing multi-selection, replace it, OR toggle off an already-selected entity.
            // Snapshot existing selection entity keys BEFORE `SelectByRay` and diff AFTER so we
            // deterministically extract ONLY the newly hit entity and NEVER mis-pick a leftover item!
            if doc_type_num != 3 {
                if let (Some((px, py)), Some(ref ext_disp)) = (screen_pos, &ext) {
                    let prev_count = sm
                        .call("GetSelectedObjectCount2", vec![VariantGuard::from_i32(-1)])?
                        .as_i32()
                        .unwrap_or(0)
                        .max(0);
                    let mut prev_keys = std::collections::HashSet::new();
                    let mut prev_order = Vec::new();
                    for idx in 1..=prev_count.min(SPEC_MAX_TARGETS as i32) {
                        if let Some(k) = snapshot_selection_item_key(&sm, ext_disp, idx) {
                            prev_keys.insert(k.clone());
                            prev_order.push(k);
                        }
                    }

                    if try_viewport_ray_pick(&doc, ext_disp, &sm, px, py, hard_deadline)? {
                        let new_count = sm
                            .call("GetSelectedObjectCount2", vec![VariantGuard::from_i32(-1)])?
                            .as_i32()
                            .unwrap_or(0)
                            .max(0);
                        let mut added_idx = None;
                        let mut after_keys = Vec::new();
                        for idx in 1..=new_count.min(SPEC_MAX_TARGETS as i32) {
                            if let Some(k) = snapshot_selection_item_key(&sm, ext_disp, idx) {
                                if !prev_keys.contains(&k) && added_idx.is_none() {
                                    added_idx = Some(idx);
                                }
                                after_keys.push((idx, k));
                            }
                        }

                        if let Some(idx) = added_idx {
                            sel_start = idx;
                            sel_end = idx;
                            pick_mode = "viewport_ray".to_string();
                        } else if new_count > prev_count && new_count > 0 {
                            sel_start = new_count;
                            sel_end = new_count;
                            pick_mode = "viewport_ray".to_string();
                        } else if new_count == 1 && prev_count <= 1 {
                            sel_start = 1;
                            sel_end = 1;
                            pick_mode = "viewport_ray".to_string();
                        } else if new_count < prev_count {
                            // Physical Ctrl toggled off an already-selected entity in SolidWorks.
                            // Re-select nothing wrong; leave targets empty so we never mis-pick a leftover item.
                            log::info!(
                                "SolidWorks ViewportRayOnly: clicked already-selected entity (toggled {prev_count} -> {new_count}); skipping leftover items"
                            );
                        } else if !after_keys.is_empty() && prev_order.is_empty() {
                            sel_start = 1;
                            sel_end = 1;
                            pick_mode = "viewport_ray".to_string();
                        }
                    }
                }
            }
        }
        SwPickMode::FeatureTreeClick => {
            // Forward click to FeatureManager Tree node so clicking a new feature during Pen mode
            // selects and appends THAT exact feature instead of repeating the old selection.
            if let Some((px, py)) = screen_pos {
                if try_feature_tree_click_pick(&doc, &sm, px, py)? {
                    sel_end = sm
                        .call("GetSelectedObjectCount2", vec![VariantGuard::from_i32(-1)])?
                        .as_i32()
                        .unwrap_or(0);
                    if sel_end > 0 {
                        pick_mode = "feature_tree_click".to_string();
                    }
                }
            }
        }
        SwPickMode::Auto => {
            sel_end = sm
                .call("GetSelectedObjectCount2", vec![VariantGuard::from_i32(-1)])?
                .as_i32()
                .unwrap_or(0);
            if sel_end > 0 {
                pick_mode = "current_selection".to_string();
            } else if doc_type_num != 3 {
                if let (Some((px, py)), Some(ref ext_disp)) = (screen_pos, &ext) {
                    if try_viewport_ray_pick(&doc, ext_disp, &sm, px, py, hard_deadline)? {
                        sel_end = sm
                            .call("GetSelectedObjectCount2", vec![VariantGuard::from_i32(-1)])?
                            .as_i32()
                            .unwrap_or(0);
                        if sel_end > 0 {
                            pick_mode = "viewport_ray".to_string();
                        }
                    }
                }
            }
        }
    }

    if pick_mode.is_empty() {
        pick_mode = "document_only".to_string();
    }

    // Keep a 450ms safety cushion before `hard_deadline` for optional metadata stages so a slow
    // `GetBox(0)` / `CustomPropertyManager` / `GetWhatsWrong` call can NEVER cause an already-picked
    // face or feature to blow the outer `recv_timeout` and degrade to `TimedOut`!
    let metadata_deadline = hard_deadline
        .checked_sub(Duration::from_millis(450))
        .unwrap_or(started);

    // -------------------------------------------------------------------------
    // Equation Manager (lazy: count is O(1); map scanned only if targets need it)
    // -------------------------------------------------------------------------
    let eq_mgr = if Instant::now() < metadata_deadline {
        doc.get_disp("GetEquationMgr")
    } else {
        None
    };
    let eq_count = eq_mgr
        .as_ref()
        .and_then(|em| em.get_i32("GetCount"))
        .unwrap_or(0)
        .max(0) as usize;

    let mut eq_map_cache: Option<HashMap<String, String>> = None;
    let mut eq_elapsed_ms = 0u64;

    // -------------------------------------------------------------------------
    // Layer 1 + Layer 2: Universal Envelope + 8 Family Enrichers (max 30 items)
    // -------------------------------------------------------------------------
    let raw_count = if sel_end >= sel_start && sel_start >= 1 {
        (sel_end - sel_start + 1) as usize
    } else {
        0
    };
    let limit = raw_count.min(SPEC_MAX_TARGETS);
    let mut targets = Vec::with_capacity(limit);

    for offset in 0..limit {
        if Instant::now() >= hard_deadline {
            break;
        }
        let i = sel_start + offset as i32;
        let type_id = sm
            .call(
                "GetSelectedObjectType3",
                vec![VariantGuard::from_i32(i), VariantGuard::from_i32(-1)],
            )
            .ok()
            .and_then(|v| v.as_i32())
            .unwrap_or(0);
        let obj = sm
            .call(
                "GetSelectedObject6",
                vec![VariantGuard::from_i32(i), VariantGuard::from_i32(-1)],
            )
            .ok()
            .and_then(|v| v.as_dispatch());
        let Some(obj) = obj else {
            continue;
        };

        let (select_type_name, family) = sw_select_type_meta(type_id);
        let mark = sm
            .call("GetSelectedObjectMark", vec![VariantGuard::from_i32(i)])
            .ok()
            .and_then(|v| v.as_i32())
            .unwrap_or(0);

        let point_mm = sm
            .call(
                "GetSelectionPoint2",
                vec![VariantGuard::from_i32(i), VariantGuard::from_i32(-1)],
            )
            .ok()
            .and_then(|v| v.as_f64_vec())
            .filter(|pts| pts.len() >= 3)
            .map(|pts| {
                [
                    round_dec(pts[0] * 1000.0, 3),
                    round_dec(pts[1] * 1000.0, 3),
                    round_dec(pts[2] * 1000.0, 3),
                ]
            });

        let persist_ref_b64 = ext.as_ref().and_then(|e| {
            let bytes = e
                .call("GetPersistReference3", vec![VariantGuard::from_dispatch(&obj.0)])
                .ok()?
                .as_bytes()?;
            if bytes.is_empty() {
                None
            } else {
                Some(base64::engine::general_purpose::STANDARD.encode(bytes))
            }
        });

        let component_chain = extract_component_chain(&sm, i);

        // Lazily populate equation map only when a target family can carry dimensions
        let needs_eq = matches!(
            family,
            SwTargetFamily::Topology | SwTargetFamily::FeatureOrRef | SwTargetFamily::Dimension
        );
        if needs_eq
            && eq_count > 0
            && eq_map_cache.is_none()
            && !skip_flags.skip_equations
            && Instant::now() < metadata_deadline
        {
            if let Some(ref em) = eq_mgr {
                let t0 = Instant::now();
                eq_map_cache = Some(read_equation_map(em, eq_count, metadata_deadline));
                eq_elapsed_ms = t0.elapsed().as_millis() as u64;
            }
        }
        let eq_ref = eq_map_cache.as_ref();

        let (display_name, details) = match family {
            SwTargetFamily::Topology => {
                enrich_topology(&doc, &obj, type_id, eq_ref, metadata_deadline)
            }
            SwTargetFamily::SketchEntity => enrich_sketch_entity(&obj, type_id),
            SwTargetFamily::FeatureOrRef => {
                enrich_feature_or_ref(&obj, eq_ref, metadata_deadline)
            }
            SwTargetFamily::Body => enrich_body(&obj),
            SwTargetFamily::Dimension => enrich_dimension(&obj, eq_ref),
            SwTargetFamily::Assembly => enrich_assembly(&obj, type_id),
            SwTargetFamily::DrawingAnnotation => enrich_drawing_annotation(&obj, type_id),
            SwTargetFamily::GenericObject => enrich_generic_object(&obj),
        };

        let resolved_point = if point_mm.is_some() {
            point_mm
        } else if type_id == 3 {
            // Vertex fallback: exact vertex coordinate in mm
            obj.call("GetPoint", Vec::new())
                .ok()
                .and_then(|v| v.as_f64_vec())
                .filter(|p| p.len() >= 3)
                .map(|p| {
                    [
                        round_dec(p[0] * 1000.0, 3),
                        round_dec(p[1] * 1000.0, 3),
                        round_dec(p[2] * 1000.0, 3),
                    ]
                })
        } else {
            None
        };

        targets.push(SwTargetV2Item {
            index: targets.len() + 1,
            select_type_id: type_id,
            select_type_name: select_type_name.to_string(),
            family,
            persist_ref_b64,
            mark,
            display_name,
            point_mm: resolved_point,
            component_chain,
            details,
        });
    }

    // -------------------------------------------------------------------------
    // Layer 3: O(1) Document-Level Context (with adaptive timing telemetry)
    // -------------------------------------------------------------------------
    let (doc_context, stage_timings) = collect_doc_context(
        &doc,
        ext.as_ref(),
        doc_title,
        doc_path,
        doc_type,
        doc_type_num,
        eq_count,
        skip_flags,
        metadata_deadline,
    );

    let total_ms = started.elapsed().as_millis() as u64;
    if let Ok(mut prof) = adaptive_profiler().lock() {
        prof.record_sample(
            &doc_key,
            total_ms,
            stage_timings.bbox_ms,
            stage_timings.custom_props_ms,
            eq_elapsed_ms,
            stage_timings.whats_wrong_ms,
        );
    }

    log::info!(
        "SolidWorks native COM pick: mode={} targets={} total={}ms (bbox={}ms props={}ms eq={}ms err={}ms)",
        pick_mode,
        targets.len(),
        total_ms,
        stage_timings.bbox_ms,
        stage_timings.custom_props_ms,
        eq_elapsed_ms,
        stage_timings.whats_wrong_ms
    );

    Ok(Some(SwTargetV2Payload {
        schema: SW_SCHEMA_V2.to_string(),
        pick_mode,
        doc_context,
        targets,
    }))
}

/// Mode B ray pick in Per-Monitor DPI-Aware physical pixels using `IModelView.Transform`
/// + `IModelView.GetVisibleBox()` + Windows `SM_CXDRAG` tolerance.
fn try_viewport_ray_pick(
    doc: &DispObj,
    ext: &DispObj,
    sm: &DispObj,
    screen_x: i32,
    screen_y: i32,
    hard_deadline: Instant,
) -> Result<bool, ComCallErr> {
    let Some(view) = doc.get_disp("ActiveView") else {
        return Ok(false);
    };
    let raw_hwnd = view
        .get_with("GetViewHWnd2", Vec::new())
        .ok()
        .and_then(|v| v.as_i64())
        .filter(|&h| h != 0)
        .or_else(|| {
            view.get_with("GetViewHWnd", Vec::new())
                .ok()
                .and_then(|v| v.as_i64())
                .filter(|&h| h != 0)
        });
    let Some(raw_hwnd) = raw_hwnd else {
        return Ok(false);
    };
    let vhwnd = HWND(raw_hwnd as *mut std::ffi::c_void);

    // Exact physical client origin & size (zero border offset, Per-Monitor DPI Aware v2)
    let (left, top, width, height) = unsafe {
        let mut client = RECT::default();
        let mut origin = POINT { x: 0, y: 0 };
        if GetClientRect(vhwnd, &mut client).is_ok()
            && ClientToScreen(vhwnd, &mut origin).as_bool()
            && (client.right - client.left) > 0
            && (client.bottom - client.top) > 0
        {
            (
                origin.x,
                origin.y,
                client.right - client.left,
                client.bottom - client.top,
            )
        } else {
            let mut wr = RECT::default();
            if GetWindowRect(vhwnd, &mut wr).is_err() {
                return Ok(false);
            }
            (wr.left, wr.top, wr.right - wr.left, wr.bottom - wr.top)
        }
    };

    if width <= 0
        || height <= 0
        || screen_x < left
        || screen_x >= left + width
        || screen_y < top
        || screen_y >= top + height
    {
        return Ok(false);
    }

    let cx = (screen_x - left) as f64;
    let cy = (screen_y - top) as f64;

    let Some(xf) = view.get_disp("Transform") else {
        return Ok(false);
    };
    let Some(arr) = xf.get("ArrayData").ok().and_then(|v| v.as_f64_vec()) else {
        return Ok(false);
    };

    // Visible model bounding box (in meters) eliminates any arbitrary ray-start Z magic constant
    let visible_box = view
        .call("GetVisibleBox", Vec::new())
        .ok()
        .and_then(|v| v.as_f64_vec())
        .filter(|b| b.len() >= 6)
        .map(|b| [b[0], b[1], b[2], b[3], b[4], b[5]]);

    let Some((mut orig, mut dir)) = invert_model_view_transform_to_ray(&arr, cx, cy, visible_box) else {
        return Ok(false);
    };

    // If SolidWorks Perspective View is enabled, refine ray origin/direction using the camera eye point.
    if view.get_bool("Perspective") == Some(true) {
        if let Some(eye) = view
            .call("GetEyePoint", Vec::new())
            .ok()
            .and_then(|v| v.as_f64_vec())
            .filter(|e| e.len() >= 3 && e.iter().all(|x| x.is_finite()))
        {
            // Project cursor point onto the focal plane (viewport z=0) and aim from eye toward it.
            let s = arr[12];
            let ux = (cx - arr[9]) / s;
            let uy = (cy - arr[10]) / s;
            let uz = -arr[11] / s;
            let focal_pt = [
                ux * arr[0] + uy * arr[1] + uz * arr[2],
                ux * arr[3] + uy * arr[4] + uz * arr[5],
                ux * arr[6] + uy * arr[7] + uz * arr[8],
            ];
            let vx = focal_pt[0] - eye[0];
            let vy = focal_pt[1] - eye[1];
            let vz = focal_pt[2] - eye[2];
            let vlen = (vx * vx + vy * vy + vz * vz).sqrt();
            if vlen > 1e-9 {
                let pdir = [vx / vlen, vy / vlen, vz / vlen];
                let dot = pdir[0] * dir[0] + pdir[1] * dir[1] + pdir[2] * dir[2];
                if dot > 0.2 {
                    orig = [eye[0], eye[1], eye[2]];
                    dir = pdir;
                }
            }
        }
    }

    let scale_px_per_m = arr[12].abs();
    // Derive edge/vertex hit radius from OS DPI-aware drag/hit metric (SM_CXDRAG)
    let px_tol = unsafe { GetSystemMetrics(SM_CXDRAG) }.max(2) as f64;
    let rad_m = px_tol / scale_px_per_m;

    // Probe Face (2) -> Edge (1) -> Vertex (3).
    // Strictly require `SelectByRay` to return `true` so clicking empty viewport background
    // NEVER falsely reports a hit from a pre-existing selection in `ISelectionMgr`.
    for type_wanted in [2i32, 1i32, 3i32] {
        if Instant::now() >= hard_deadline {
            break;
        }
        let hit = ext
            .call(
                "SelectByRay",
                vec![
                    VariantGuard::from_f64(orig[0]),
                    VariantGuard::from_f64(orig[1]),
                    VariantGuard::from_f64(orig[2]),
                    VariantGuard::from_f64(dir[0]),
                    VariantGuard::from_f64(dir[1]),
                    VariantGuard::from_f64(dir[2]),
                    VariantGuard::from_f64(rad_m),
                    VariantGuard::from_i32(type_wanted),
                    VariantGuard::from_bool(false),
                    VariantGuard::from_i32(0),
                    VariantGuard::from_i32(0),
                ],
            )?
            .as_bool()
            .unwrap_or(false);
        if hit {
            let c = sm
                .call("GetSelectedObjectCount2", vec![VariantGuard::from_i32(-1)])?
                .as_i32()
                .unwrap_or(0);
            if c > 0 {
                log::info!(
                    "SolidWorks SelectByRay hit type={type_wanted} at client=({cx:.0},{cy:.0}) scale={scale_px_per_m:.1} orig=({:.4},{:.4},{:.4}) dir=({:.3},{:.3},{:.3})",
                    orig[0], orig[1], orig[2], dir[0], dir[1], dir[2]
                );
                return Ok(true);
            }
        }
    }

    log::info!(
        "SolidWorks SelectByRay miss at client=({cx:.0},{cy:.0}) scale={scale_px_per_m:.1} orig=({:.4},{:.4},{:.4}) dir=({:.3},{:.3},{:.3})",
        orig[0], orig[1], orig[2], dir[0], dir[1], dir[2]
    );
    Ok(false)
}

/// Fast per-item identity key (`type_id:persist_ref_b64`) used before/after `SelectByRay`
/// to isolate strictly the newly clicked entity even when physical `Ctrl` is held.
fn snapshot_selection_item_key(sm: &DispObj, ext: &DispObj, index_1based: i32) -> Option<String> {
    let type_id = sm
        .call(
            "GetSelectedObjectType3",
            vec![VariantGuard::from_i32(index_1based), VariantGuard::from_i32(-1)],
        )
        .ok()
        .and_then(|v| v.as_i32())
        .unwrap_or(0);
    let obj = sm
        .call(
            "GetSelectedObject6",
            vec![VariantGuard::from_i32(index_1based), VariantGuard::from_i32(-1)],
        )
        .ok()
        .and_then(|v| v.as_dispatch())?;
    if let Some(bytes) = ext
        .call("GetPersistReference3", vec![VariantGuard::from_dispatch(&obj.0)])
        .ok()
        .and_then(|v| v.as_bytes())
        .filter(|b| !b.is_empty())
    {
        let b64 = base64::engine::general_purpose::STANDARD.encode(bytes);
        return Some(format!("{type_id}:pid:{b64}"));
    }
    let ptr = obj.0.as_raw() as usize;
    Some(format!("{type_id}:ptr:{ptr:x}"))
}

/// Selects the FeatureManager Tree item under `(screen_x, screen_y)` by clearing the prior
/// selection (preventing accidental inline rename) and posting a single client-coordinate click
/// to the tree control, then reading back `ISelectionMgr`.
fn try_feature_tree_click_pick(
    doc: &DispObj,
    sm: &DispObj,
    screen_x: i32,
    screen_y: i32,
) -> Result<bool, ComCallErr> {
    let child = unsafe { WindowFromPoint(POINT { x: screen_x, y: screen_y }) };
    if child.0.is_null() {
        return Ok(false);
    }
    let mut pt = POINT {
        x: screen_x,
        y: screen_y,
    };
    if !unsafe { ScreenToClient(child, &mut pt) }.as_bool() {
        return Ok(false);
    }

    // Clear prior selection so clicking the tree item selects it cleanly and never triggers rename.
    let _ = doc.call("ClearSelection2", vec![VariantGuard::from_bool(true)]);

    let lparam = LPARAM((((pt.y as u32) & 0xFFFF) << 16 | ((pt.x as u32) & 0xFFFF)) as isize);
    unsafe {
        let _ = PostMessageW(Some(child), WM_LBUTTONDOWN, WPARAM(0x0001), lparam);
        let _ = PostMessageW(Some(child), WM_LBUTTONUP, WPARAM(0), lparam);
    }

    // Give SolidWorks UI thread a brief window to update `ISelectionMgr`
    for _ in 0..4 {
        std::thread::sleep(Duration::from_millis(25));
        let c = sm
            .call("GetSelectedObjectCount2", vec![VariantGuard::from_i32(-1)])?
            .as_i32()
            .unwrap_or(0);
        if c > 0 {
            return Ok(true);
        }
    }

    Ok(false)
}

fn extract_component_chain(sm: &DispObj, sel_index: i32) -> Vec<String> {
    let comp = sm
        .call(
            "GetSelectedObjectsComponent4",
            vec![VariantGuard::from_i32(sel_index), VariantGuard::from_i32(-1)],
        )
        .ok()
        .and_then(|v| v.as_dispatch());
    let Some(mut cur) = comp else {
        return Vec::new();
    };
    let mut chain = Vec::new();
    let mut visited = 0usize;
    // Walk parent pointers upward to root assembly; stops naturally when GetParent() is null
    while visited < 64 {
        visited += 1;
        let name = cur.get_str("Name2");
        if !name.is_empty() {
            // SolidWorks `Name2` often returns "sub_asm-1/housing-2"; take leaf segment per parent step
            let leaf = name.rsplit('/').next().unwrap_or(&name).to_string();
            chain.push(leaf);
        }
        match cur.call("GetParent", Vec::new()).ok().and_then(|v| v.as_dispatch()) {
            Some(parent) => cur = parent,
            None => break,
        }
    }
    chain.reverse();
    chain
}

fn read_equation_map(
    eq_mgr: &DispObj,
    eq_count: usize,
    hard_deadline: Instant,
) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for k in 0..eq_count {
        if Instant::now() >= hard_deadline {
            break;
        }
        let eq_str = eq_mgr
            .get_with("Equation", vec![VariantGuard::from_i32(k as i32)])
            .ok()
            .and_then(|v| v.as_string())
            .unwrap_or_default();
        if let Some((lhs, rhs)) = parse_equation_entry(&eq_str) {
            map.insert(lhs.to_lowercase(), rhs);
        }
    }
    map
}

fn lookup_equation_rhs(
    eq_map: Option<&HashMap<String, String>>,
    full_name: &str,
) -> Option<String> {
    let map = eq_map?;
    let key = full_name.trim().to_lowercase();
    if let Some(rhs) = map.get(&key) {
        return Some(rhs.clone());
    }
    // Some SolidWorks dimension FullNames append `@PartName.SLDPRT` — match prefix `D1@Feature` too
    let parts: Vec<&str> = key.split('@').collect();
    if parts.len() >= 2 {
        let short = format!("{}@{}", parts[0], parts[1]);
        if let Some(rhs) = map.get(&short) {
            return Some(rhs.clone());
        }
    }
    None
}

fn extract_feature_dimensions_json(
    feat: &DispObj,
    eq_map: Option<&HashMap<String, String>>,
    hard_deadline: Instant,
) -> Vec<Value> {
    let mut out = Vec::new();
    let mut cur = feat
        .call("GetFirstDisplayDimension", Vec::new())
        .ok()
        .and_then(|v| v.as_dispatch());
    let mut seen_names = std::collections::HashSet::new();

    while let Some(dd) = cur {
        if Instant::now() >= hard_deadline {
            break;
        }
        if let Some(d) = dd
            .call("GetDimension2", vec![VariantGuard::from_i32(0)])
            .ok()
            .and_then(|v| v.as_dispatch())
        {
            let full_name = d.get_str("FullName");
            if !full_name.is_empty() && !seen_names.insert(full_name.clone()) {
                break;
            }
            let name = d.get_str("Name");
            let sv = d.get_f64("SystemValue").unwrap_or(0.0);
            let dtype = d.get_i32("GetType").unwrap_or(0);
            let (value, unit) = if dtype == 1 {
                (round_dec(sv * 180.0 / std::f64::consts::PI, 3), "deg")
            } else {
                (round_dec(sv * 1000.0, 4), "mm")
            };
            let eq_rhs = lookup_equation_rhs(eq_map, &full_name);
            out.push(json!({
                "full_name": full_name,
                "name": name,
                "value": value,
                "unit": unit,
                "equation_rhs": eq_rhs,
            }));
        }
        cur = feat
            .call(
                "GetNextDisplayDimension",
                vec![VariantGuard::from_dispatch(&dd.0)],
            )
            .ok()
            .and_then(|v| v.as_dispatch());
    }
    out
}

struct FeatureSummary {
    name: String,
    feature_type: String,
    suppressed: bool,
    error_code: i32,
    is_warning: bool,
    face_count: i32,
    parents: Vec<String>,
    children: Vec<String>,
    dimensions: Vec<Value>,
}

fn inspect_feature(
    feat: &DispObj,
    eq_map: Option<&HashMap<String, String>>,
    hard_deadline: Instant,
) -> FeatureSummary {
    let name = feat.get_str("Name");
    let feature_type = feat.get_str("GetTypeName2");

    let suppressed = feat
        .call(
            "IsSuppressed2",
            vec![VariantGuard::from_i32(1), VariantGuard::empty()],
        )
        .ok()
        .and_then(|v| {
            v.as_bool_vec()
                .and_then(|bv| bv.first().copied())
                .or_else(|| v.as_bool())
        })
        .or_else(|| feat.get_bool("IsSuppressed"))
        .unwrap_or(false);

    let mut is_warn_raw = VARIANT_FALSE;
    let error_code = feat
        .call("GetErrorCode2", vec![VariantGuard::byref_bool(&mut is_warn_raw)])
        .ok()
        .and_then(|v| v.as_i32())
        .unwrap_or(0);
    let is_warning = is_warn_raw.0 != 0;

    let face_count = feat.get_i32("GetFaceCount").unwrap_or(0);

    let parents = feat
        .call("GetParents", Vec::new())
        .ok()
        .and_then(|v| v.as_dispatch_vec())
        .map(|vec| {
            vec.into_iter()
                .take(SPEC_MAX_PARENTS_CHILDREN)
                .map(|p| p.get_str("Name"))
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let children = feat
        .call("GetChildren", Vec::new())
        .ok()
        .and_then(|v| v.as_dispatch_vec())
        .map(|vec| {
            vec.into_iter()
                .take(SPEC_MAX_PARENTS_CHILDREN)
                .map(|c| c.get_str("Name"))
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let dimensions = extract_feature_dimensions_json(feat, eq_map, hard_deadline);

    FeatureSummary {
        name,
        feature_type,
        suppressed,
        error_code,
        is_warning,
        face_count,
        parents,
        children,
        dimensions,
    }
}

fn box_m_to_mm_json(raw: Option<Vec<f64>>) -> Option<Value> {
    let b = raw?;
    if b.len() < 6 {
        return None;
    }
    Some(json!([
        round_dec(b[0] * 1000.0, 3),
        round_dec(b[1] * 1000.0, 3),
        round_dec(b[2] * 1000.0, 3),
        round_dec(b[3] * 1000.0, 3),
        round_dec(b[4] * 1000.0, 3),
        round_dec(b[5] * 1000.0, 3),
    ]))
}

// =============================================================================
// Family 1: Topology (FACE, EDGE, VERTEX, LOOP, SILHOUETTES)
// =============================================================================
fn enrich_topology(
    doc: &DispObj,
    obj: &DispObj,
    type_id: i32,
    eq_map: Option<&HashMap<String, String>>,
    hard_deadline: Instant,
) -> (String, Value) {
    let mut map = Map::new();
    let entity_name = doc
        .call("GetEntityName", vec![VariantGuard::from_dispatch(&obj.0)])
        .ok()
        .and_then(|v| v.as_string())
        .unwrap_or_default();
    map.insert("entity_name".into(), json!(entity_name));

    let mut owner_feat: Option<DispObj> = None;
    let mut owner_body_name = String::new();

    match type_id {
        2 => {
            // swSelFACES
            owner_feat = obj.get_disp("GetFeature");
            if let Some(body) = obj.get_disp("GetBody") {
                owner_body_name = body.get_str("Name");
            }
            if let Some(nrm) = obj.get("Normal").ok().and_then(|v| v.as_f64_vec()) {
                if nrm.len() >= 3 {
                    let nv = [round_dec(nrm[0], 4), round_dec(nrm[1], 4), round_dec(nrm[2], 4)];
                    map.insert("normal".into(), json!(nv));
                    if let Some(dir) = classify_semantic_direction(nv) {
                        map.insert("dir".into(), json!(dir));
                    }
                }
            }
            if let Some(surf) = obj.get_disp("GetSurface") {
                if surf.get_bool("IsPlane").unwrap_or(false) {
                    map.insert("surface".into(), json!("Plane"));
                } else if surf.get_bool("IsCylinder").unwrap_or(false) {
                    map.insert("surface".into(), json!("Cylinder"));
                    if let Some(cp) = surf.get("CylinderParams").ok().and_then(|v| v.as_f64_vec()) {
                        if cp.len() >= 7 {
                            map.insert("radius_mm".into(), json!(round_dec(cp[6] * 1000.0, 3)));
                        }
                    }
                } else if surf.get_bool("IsCone").unwrap_or(false) {
                    map.insert("surface".into(), json!("Cone"));
                } else if surf.get_bool("IsSphere").unwrap_or(false) {
                    map.insert("surface".into(), json!("Sphere"));
                    if let Some(sp) = surf.get("SphereParams").ok().and_then(|v| v.as_f64_vec()) {
                        if sp.len() >= 4 {
                            map.insert("radius_mm".into(), json!(round_dec(sp[3] * 1000.0, 3)));
                        }
                    }
                } else if surf.get_bool("IsTorus").unwrap_or(false) {
                    map.insert("surface".into(), json!("Torus"));
                } else {
                    map.insert("surface".into(), json!("Surface"));
                }
            }
            if let Some(area) = obj.get_f64("GetArea") {
                let area_mm2 = round_dec(area * 1e6, 2);
                if area_mm2 > 0.0 {
                    map.insert("area_mm2".into(), json!(area_mm2));
                }
            }
            if let Some(bb) = box_m_to_mm_json(obj.call("GetBox", Vec::new()).ok().and_then(|v| v.as_f64_vec())) {
                map.insert("bbox_mm".into(), bb);
            }
        }
        1 => {
            // swSelEDGES
            if let Some(adj) = obj
                .call("GetTwoAdjacentFaces2", Vec::new())
                .ok()
                .and_then(|v| v.as_dispatch_vec())
            {
                for af in adj {
                    if owner_body_name.is_empty() {
                        if let Some(b) = af.get_disp("GetBody") {
                            owner_body_name = b.get_str("Name");
                        }
                    }
                    if owner_feat.is_none() {
                        owner_feat = af.get_disp("GetFeature");
                    }
                    if owner_feat.is_some() && !owner_body_name.is_empty() {
                        break;
                    }
                }
            }
            if let Some(crv) = obj.get_disp("GetCurve") {
                if crv.get_bool("IsLine").unwrap_or(false) {
                    map.insert("curve".into(), json!("Line"));
                } else if crv.get_bool("IsCircle").unwrap_or(false) {
                    map.insert("curve".into(), json!("Circle"));
                    if let Some(cp) = crv.get("CircleParams").ok().and_then(|v| v.as_f64_vec()) {
                        if cp.len() >= 7 {
                            map.insert("radius_mm".into(), json!(round_dec(cp[6] * 1000.0, 3)));
                        }
                    }
                } else if crv.get_bool("IsEllipse").unwrap_or(false) {
                    map.insert("curve".into(), json!("Ellipse"));
                } else if crv.get_bool("IsBcurve").unwrap_or(false) {
                    map.insert("curve".into(), json!("Spline"));
                } else {
                    map.insert("curve".into(), json!("Curve"));
                }
                if let Some(cp3) = obj.get_disp("GetCurveParams3") {
                    if let (Some(u0), Some(u1)) = (cp3.get_f64("UMinValue"), cp3.get_f64("UMaxValue")) {
                        if let Some(len_m) = crv
                            .call(
                                "GetLength3",
                                vec![VariantGuard::from_f64(u0), VariantGuard::from_f64(u1)],
                            )
                            .ok()
                            .and_then(|v| v.as_f64())
                        {
                            let len_mm = round_dec(len_m * 1000.0, 3);
                            if len_mm > 0.0 {
                                map.insert("length_mm".into(), json!(len_mm));
                            }
                        }
                    }
                }
            }
        }
        3 => {
            // swSelVERTICES
            if let Some(faces) = obj
                .call("GetAdjacentFaces", Vec::new())
                .ok()
                .and_then(|v| v.as_dispatch_vec())
            {
                if let Some(first_face) = faces.first() {
                    if let Some(b) = first_face.get_disp("GetBody") {
                        owner_body_name = b.get_str("Name");
                    }
                    owner_feat = first_face.get_disp("GetFeature");
                }
            }
        }
        24 => {
            // swSelLOOP
            if let Some(face) = obj.get_disp("GetFace") {
                owner_feat = face.get_disp("GetFeature");
                if let Some(b) = face.get_disp("GetBody") {
                    owner_body_name = b.get_str("Name");
                }
            }
            if let Some(is_outer) = obj.get_bool("IsOuter") {
                map.insert("is_outer_loop".into(), json!(is_outer));
            }
            if let Some(ec) = obj.get_i32("GetEdgeCount") {
                map.insert("edge_count".into(), json!(ec));
            }
        }
        _ => {}
    }

    let mut owner_feature_name = String::new();
    if let Some(ref feat) = owner_feat {
        let fs = inspect_feature(feat, eq_map, hard_deadline);
        owner_feature_name = fs.name.clone();
        map.insert("owner_feature".into(), json!(fs.name));
        if !fs.feature_type.is_empty() {
            map.insert("feature_type".into(), json!(fs.feature_type));
        }
        if !fs.dimensions.is_empty() {
            map.insert("dimensions".into(), Value::Array(fs.dimensions));
        }
        if !fs.parents.is_empty() {
            map.insert("parents".into(), json!(fs.parents));
        }
        if !fs.children.is_empty() {
            map.insert("children".into(), json!(fs.children));
        }
    } else {
        map.insert("owner_feature".into(), json!(""));
    }
    map.insert("owner_body".into(), json!(owner_body_name));

    let display_name = if !entity_name.is_empty() {
        entity_name
    } else if !owner_feature_name.is_empty() {
        owner_feature_name
    } else if !owner_body_name.is_empty() {
        owner_body_name
    } else {
        sw_select_type_meta(type_id).0.to_string()
    };
    (display_name, Value::Object(map))
}

// =============================================================================
// Family 2: Sketch Entity (SKETCHSEGMENT, SKETCHPOINT, SKETCHTEXT, REGION, CONTOUR)
// =============================================================================
fn enrich_sketch_entity(obj: &DispObj, type_id: i32) -> (String, Value) {
    let mut map = Map::new();

    let sketch = obj.get_disp("GetSketch").or_else(|| obj.get_disp("Sketch"));
    let mut sketch_name = String::new();
    if let Some(ref sk) = sketch {
        // ISketch is also an IFeature (or accessible via Feature)
        sketch_name = sk.get_str("Name");
        if sketch_name.is_empty() {
            if let Some(f) = sk.get_disp("GetFeature") {
                sketch_name = f.get_str("Name");
            }
        }
        if let Some(cs) = sk.get_i32("GetConstrainedStatus") {
            let status_str = match cs {
                0 => "UnderConstrained",
                1 => "FullyConstrained",
                2 => "OverConstrained",
                3 => "NoSolution",
                4 => "InvalidSolution",
                _ => "Unknown",
            };
            map.insert("constrained_status".into(), json!(status_str));
        }
    }
    map.insert("sketch_name".into(), json!(sketch_name));

    let mut seg_label = String::new();
    if matches!(type_id, 10 | 25) {
        // ISketchSegment
        let st = obj.get_i32("GetType").unwrap_or(-1);
        let seg_type = match st {
            0 => "Line",
            1 => "Arc",
            2 => "Ellipse",
            3 => "Spline",
            4 => "Text",
            5 => "Parabola",
            _ => "Segment",
        };
        seg_label = seg_type.to_string();
        map.insert("segment_type".into(), json!(seg_type));
        map.insert(
            "is_construction".into(),
            json!(obj.get_bool("ConstructionGeometry").unwrap_or(false)),
        );
        if let Some(len_m) = obj.get_f64("GetLength") {
            let len_mm = round_dec(len_m * 1000.0, 3);
            if len_mm > 0.0 {
                map.insert("length_mm".into(), json!(len_mm));
            }
        }
        // Line endpoints
        if st == 0 {
            if let Some(sp) = obj.get_disp("GetStartPoint2") {
                if let (Some(x), Some(y), Some(z)) = (sp.get_f64("X"), sp.get_f64("Y"), sp.get_f64("Z")) {
                    map.insert(
                        "start_mm".into(),
                        json!([round_dec(x * 1000.0, 3), round_dec(y * 1000.0, 3), round_dec(z * 1000.0, 3)]),
                    );
                }
            }
            if let Some(ep) = obj.get_disp("GetEndPoint2") {
                if let (Some(x), Some(y), Some(z)) = (ep.get_f64("X"), ep.get_f64("Y"), ep.get_f64("Z")) {
                    map.insert(
                        "end_mm".into(),
                        json!([round_dec(x * 1000.0, 3), round_dec(y * 1000.0, 3), round_dec(z * 1000.0, 3)]),
                    );
                }
            }
        } else if st == 1 {
            // Arc / Circle
            if let Some(is_c) = obj.get_bool("IsCircle") {
                if is_c {
                    seg_label = "Circle".to_string();
                    map.insert("segment_type".into(), json!("Circle"));
                }
            }
            if let Some(r_m) = obj.get_f64("GetRadius") {
                map.insert("radius_mm".into(), json!(round_dec(r_m * 1000.0, 3)));
            }
            if let Some(cp) = obj.get_disp("GetCenterPoint2") {
                if let (Some(x), Some(y), Some(z)) = (cp.get_f64("X"), cp.get_f64("Y"), cp.get_f64("Z")) {
                    map.insert(
                        "center_mm".into(),
                        json!([round_dec(x * 1000.0, 3), round_dec(y * 1000.0, 3), round_dec(z * 1000.0, 3)]),
                    );
                }
            }
        }
        // Existing geometric relations
        if let Some(rels) = obj
            .call("GetRelations", Vec::new())
            .ok()
            .and_then(|v| v.as_dispatch_vec())
        {
            let names: Vec<String> = rels
                .into_iter()
                .filter_map(|r| r.get_i32("GetRelationType").map(sketch_relation_type_name))
                .collect();
            if !names.is_empty() {
                map.insert("relations".into(), json!(names));
            }
        }
    } else if matches!(type_id, 11 | 26) {
        seg_label = "Point".to_string();
        map.insert("segment_type".into(), json!("Point"));
        if let (Some(x), Some(y), Some(z)) = (obj.get_f64("X"), obj.get_f64("Y"), obj.get_f64("Z")) {
            map.insert(
                "point_mm".into(),
                json!([round_dec(x * 1000.0, 3), round_dec(y * 1000.0, 3), round_dec(z * 1000.0, 3)]),
            );
        }
    } else if type_id == 38 {
        seg_label = "SketchText".to_string();
        map.insert("segment_type".into(), json!("Text"));
        let txt = obj.get_str("Text");
        if !txt.is_empty() {
            map.insert("text".into(), json!(txt));
        }
    } else if type_id == 96 {
        seg_label = "SketchRegion".to_string();
        map.insert("segment_type".into(), json!("Region"));
    } else if type_id == 97 {
        seg_label = "SketchContour".to_string();
        map.insert("segment_type".into(), json!("Contour"));
        if let Some(closed) = obj.get_bool("IsClosed") {
            map.insert("is_closed".into(), json!(closed));
        }
    }

    let display_name = if !sketch_name.is_empty() && !seg_label.is_empty() {
        format!("{sketch_name} · {seg_label}")
    } else if !sketch_name.is_empty() {
        sketch_name
    } else if !seg_label.is_empty() {
        seg_label
    } else {
        sw_select_type_meta(type_id).0.to_string()
    };
    (display_name, Value::Object(map))
}

fn sketch_relation_type_name(rel_id: i32) -> String {
    match rel_id {
        0 => "Horizontal",
        1 => "Vertical",
        2 => "Collinear",
        3 => "Coradial",
        4 => "Perpendicular",
        5 => "Parallel",
        6 => "Tangent",
        7 => "Concentric",
        8 => "Coincident",
        9 => "Symmetric",
        10 => "AtMiddle",
        11 => "AtIntersect",
        12 => "AtPierce",
        13 => "MergePoints",
        14 => "EqualLength",
        15 => "Fixed",
        16 => "EqualCurvature",
        _ => return format!("Relation({rel_id})"),
    }
    .to_string()
}

// =============================================================================
// Family 3: Feature or Reference Geometry
// =============================================================================
fn enrich_feature_or_ref(
    obj: &DispObj,
    eq_map: Option<&HashMap<String, String>>,
    hard_deadline: Instant,
) -> (String, Value) {
    let fs = inspect_feature(obj, eq_map, hard_deadline);
    let display = if !fs.name.is_empty() {
        fs.name.clone()
    } else {
        fs.feature_type.clone()
    };
    let details = json!({
        "feature_name": fs.name,
        "feature_type": fs.feature_type,
        "suppressed": fs.suppressed,
        "error_code": fs.error_code,
        "is_warning": fs.is_warning,
        "face_count": fs.face_count,
        "dimensions": fs.dimensions,
        "parents": fs.parents,
        "children": fs.children,
    });
    (display, details)
}

// =============================================================================
// Family 4: Solid / Surface Body
// =============================================================================
fn enrich_body(obj: &DispObj) -> (String, Value) {
    let name = obj.get_str("Name");
    let bt = obj.get_i32("GetType").unwrap_or(-1);
    let body_type = match bt {
        0 => "Solid",
        1 => "Sheet",
        2 => "Wire",
        3 => "Minimum",
        4 => "General",
        _ => "Body",
    };
    let is_sheet_metal = obj.get_bool("IsSheetMetal").unwrap_or(false);
    let face_count = obj.get_i32("GetFaceCount").unwrap_or(0);
    let edge_count = obj.get_i32("GetEdgeCount").unwrap_or(0);
    let bbox_mm = box_m_to_mm_json(obj.call("GetBodyBox", Vec::new()).ok().and_then(|v| v.as_f64_vec()));

    let mut db_slot = VARIANT::default();
    let material = obj
        .call(
            "GetMaterialPropertyName2",
            vec![VariantGuard::from_str(""), VariantGuard::byref_variant(&mut db_slot)],
        )
        .ok()
        .and_then(|v| v.as_string())
        .unwrap_or_default();
    unsafe {
        let _ = VariantClear(&mut db_slot);
    }

    let mut map = Map::new();
    map.insert("body_name".into(), json!(name));
    map.insert("body_type".into(), json!(body_type));
    map.insert("is_sheet_metal".into(), json!(is_sheet_metal));
    map.insert("face_count".into(), json!(face_count));
    map.insert("edge_count".into(), json!(edge_count));
    if let Some(bb) = bbox_mm {
        map.insert("bbox_mm".into(), bb);
    }
    if !material.is_empty() {
        map.insert("material".into(), json!(material));
    }

    let display = if !name.is_empty() {
        name
    } else {
        body_type.to_string()
    };
    (display, Value::Object(map))
}

// =============================================================================
// Family 5: Dimension (DisplayDimension -> IDimension + Tolerance + Equation)
// =============================================================================
fn enrich_dimension(obj: &DispObj, eq_map: Option<&HashMap<String, String>>) -> (String, Value) {
    let dim = obj
        .call("GetDimension2", vec![VariantGuard::from_i32(0)])
        .ok()
        .and_then(|v| v.as_dispatch())
        .unwrap_or_else(|| obj.clone());

    let full_name = dim.get_str("FullName");
    let name = dim.get_str("Name");
    let sv = dim.get_f64("SystemValue").unwrap_or(0.0);
    let dtype = dim.get_i32("GetType").unwrap_or(0);
    let (value, unit) = if dtype == 1 {
        (round_dec(sv * 180.0 / std::f64::consts::PI, 3), "deg")
    } else {
        (round_dec(sv * 1000.0, 4), "mm")
    };

    // swDimensionDrivenState_e: 1 = Driven (reference), 2 = Driving
    let driven_state = dim.get_i32("DrivenState").unwrap_or(2);
    let is_driven = driven_state == 1;

    let mut tol_val = Value::Null;
    if let Some(tol) = dim.get_disp("Tolerance") {
        let tol_type_id = tol.get_i32("Type").unwrap_or(0);
        if tol_type_id != 0 {
            let tol_type = match tol_type_id {
                1 => "Basic",
                2 => "Bilateral",
                3 => "Limit",
                4 => "Symmetric",
                5 => "Min",
                6 => "Max",
                7 => "Fit",
                8 => "FitWithTol",
                9 => "FitTolOnly",
                _ => "Other",
            };
            let mut upper_mm = 0.0;
            let mut lower_mm = 0.0;
            if let Ok(min_res) = tol.call("GetMinValue", Vec::new()) {
                if let Some(v) = min_res.as_f64() {
                    lower_mm = round_dec(v * 1000.0, 4);
                }
            }
            if let Ok(max_res) = tol.call("GetMaxValue", Vec::new()) {
                if let Some(v) = max_res.as_f64() {
                    upper_mm = round_dec(v * 1000.0, 4);
                }
            }
            tol_val = json!({
                "type": tol_type,
                "upper_mm": upper_mm,
                "lower_mm": lower_mm,
            });
        }
    }

    let eq_rhs = lookup_equation_rhs(eq_map, &full_name);
    let display = if !full_name.is_empty() {
        format!("{full_name} = {value} {unit}")
    } else {
        format!("{value} {unit}")
    };

    let details = json!({
        "full_name": full_name,
        "name": name,
        "value": value,
        "unit": unit,
        "is_driven": is_driven,
        "tolerance": tol_val,
        "equation_rhs": eq_rhs,
    });
    (display, details)
}

// =============================================================================
// Family 6: Assembly (COMPONENT, MATE, COMPPATTERN, MATEGROUP)
// =============================================================================
fn enrich_assembly(obj: &DispObj, type_id: i32) -> (String, Value) {
    if type_id == 20 {
        // swSelCOMPONENTS -> IComponent2
        let name = obj.get_str("Name2");
        let path = obj.get_str("GetPathName");
        let config = obj.get_str("ReferencedConfiguration");
        let suppressed = obj.get_bool("IsSuppressed").unwrap_or(false);
        let is_fixed = obj.get_bool("IsFixed").unwrap_or(false);
        let is_virtual = obj.get_bool("IsVirtual").unwrap_or(false);
        let child_count = obj
            .call("GetChildren", Vec::new())
            .ok()
            .and_then(|v| v.as_dispatch_vec())
            .map(|v| v.len())
            .unwrap_or(0);
        let display = name.rsplit('/').next().unwrap_or(&name).to_string();
        return (
            display,
            json!({
                "name": name,
                "path": path,
                "config": config,
                "suppressed": suppressed,
                "is_fixed": is_fixed,
                "is_virtual": is_virtual,
                "child_count": child_count,
            }),
        );
    }

    // Mates (21 / 34) & MateGroup / ComponentPattern
    let feat_name = obj.get_str("Name");
    let feat_type = obj.get_str("GetTypeName2");
    let mate_obj = obj.get_disp("GetSpecificFeature2").unwrap_or_else(|| obj.clone());
    let mate_type_id = mate_obj.get_i32("Type").unwrap_or(-1);
    let mate_type = match mate_type_id {
        0 => "Coincident",
        1 => "Concentric",
        2 => "Perpendicular",
        3 => "Parallel",
        4 => "Tangent",
        5 => "Distance",
        6 => "Angle",
        8 => "Symmetric",
        9 => "CamFollower",
        10 => "Gear",
        11 => "Width",
        12 => "LockToSketch",
        13 => "RackPinion",
        14 => "MaxMate",
        15 => "Path",
        16 => "Lock",
        17 => "Screw",
        18 => "LinearCoupler",
        19 => "UniversalJoint",
        20 => "Coordinate",
        21 => "Slot",
        22 => "Hinge",
        _ => feat_type.as_str(),
    };
    let align_id = mate_obj.get_i32("Alignment").unwrap_or(-1);
    let alignment = match align_id {
        0 => "Aligned",
        1 => "AntiAligned",
        2 => "Closest",
        _ => "Unknown",
    };

    let suppressed = obj
        .call(
            "IsSuppressed2",
            vec![VariantGuard::from_i32(1), VariantGuard::empty()],
        )
        .ok()
        .and_then(|v| v.as_bool_vec().and_then(|b| b.first().copied()).or_else(|| v.as_bool()))
        .unwrap_or(false);
    let mut is_warn = VARIANT_FALSE;
    let error_code = obj
        .call("GetErrorCode2", vec![VariantGuard::byref_bool(&mut is_warn)])
        .ok()
        .and_then(|v| v.as_i32())
        .unwrap_or(0);

    let mut mate_entities = Vec::new();
    let entity_count = mate_obj.get_i32("GetMateEntityCount").unwrap_or(0).max(0);
    for ei in 0..entity_count {
        if let Some(me) = mate_obj
            .call("MateEntity", vec![VariantGuard::from_i32(ei)])
            .ok()
            .and_then(|v| v.as_dispatch())
        {
            let comp_name = me
                .get_disp("ReferenceComponent")
                .map(|c| c.get_str("Name2"))
                .unwrap_or_default();
            let ent_type = me.get_i32("ReferenceType2").unwrap_or(-1);
            mate_entities.push(json!({
                "component": comp_name,
                "reference_type_id": ent_type,
            }));
        }
    }

    let display = if !feat_name.is_empty() {
        feat_name.clone()
    } else {
        mate_type.to_string()
    };
    (
        display,
        json!({
            "name": feat_name,
            "mate_type": mate_type,
            "alignment": alignment,
            "suppressed": suppressed,
            "error_code": error_code,
            "mate_entities": mate_entities,
        }),
    )
}

// =============================================================================
// Family 7: Drawing Views, Sheets & Annotations
// =============================================================================
fn enrich_drawing_annotation(obj: &DispObj, type_id: i32) -> (String, Value) {
    let mut map = Map::new();
    match type_id {
        12 => {
            // swSelDRAWINGVIEWS -> IView
            let view_name = obj.get_str("Name");
            let scale = obj
                .get("ScaleRatio")
                .ok()
                .and_then(|v| v.as_f64_vec())
                .filter(|s| s.len() >= 2)
                .map(|s| format!("{}:{}", round_dec(s[0], 3), round_dec(s[1], 3)))
                .unwrap_or_default();
            let ref_path = obj
                .get_disp("ReferencedDocument")
                .map(|d| d.get_str("GetPathName"))
                .unwrap_or_default();
            let ref_cfg = obj.get_str("ReferencedConfiguration");
            map.insert("view_name".into(), json!(view_name));
            if !scale.is_empty() {
                map.insert("scale".into(), json!(scale));
            }
            map.insert("referenced_model_path".into(), json!(ref_path));
            map.insert("referenced_config".into(), json!(ref_cfg));
            (view_name, Value::Object(map))
        }
        19 => {
            // swSelSHEETS -> ISheet
            let name = obj.get_str("GetName");
            map.insert("view_name".into(), json!(name));
            map.insert("symbol_type".into(), json!("Sheet"));
            (name, Value::Object(map))
        }
        13 => {
            // swSelGTOLS -> IGtol
            let frame_count = obj.get_i32("GetFrameCount").unwrap_or(0).max(0);
            let mut frames = Vec::new();
            for fi in 1..=frame_count {
                let sym = obj
                    .call("GetFrameSymbols3", vec![VariantGuard::from_i32(fi)])
                    .ok()
                    .and_then(|v| v.as_string())
                    .unwrap_or_default();
                let vals = obj
                    .call("GetFrameValues", vec![VariantGuard::from_i32(fi)])
                    .ok()
                    .and_then(|v| v.as_string_vec())
                    .unwrap_or_default();
                frames.push(json!({ "frame": fi, "symbol": sym, "values": vals }));
            }
            let text = obj.get_str("GetText");
            map.insert("symbol_type".into(), json!("GTol"));
            if !text.is_empty() {
                map.insert("text".into(), json!(text));
            }
            map.insert("gtol_frames".into(), Value::Array(frames));
            let display = if !text.is_empty() {
                text
            } else {
                "形位公差 (GTol)".to_string()
            };
            (display, Value::Object(map))
        }
        15 => {
            // swSelNOTES -> INote
            let text = obj.get_str("GetText");
            map.insert("symbol_type".into(), json!("Note"));
            map.insert("text".into(), json!(text));
            let display = if !text.is_empty() {
                text.lines().next().unwrap_or(&text).trim().to_string()
            } else {
                "注释 (Note)".to_string()
            };
            (display, Value::Object(map))
        }
        _ => {
            let sym_type = sw_select_type_meta(type_id).0;
            let text = obj
                .get_str("GetText")
                .or_if_empty(|| obj.get_str("GetLabel"))
                .or_if_empty(|| obj.get_str("Name"));
            map.insert("symbol_type".into(), json!(sym_type));
            if !text.is_empty() {
                map.insert("text".into(), json!(text));
            }
            let display = if !text.is_empty() {
                text
            } else {
                sym_type.to_string()
            };
            (display, Value::Object(map))
        }
    }
}

// =============================================================================
// Family 8: Generic Object Fallback (zero errors, zero dropped objects)
// =============================================================================
fn enrich_generic_object(obj: &DispObj) -> (String, Value) {
    let com_type_hint = obj.com_type_name().unwrap_or_default();
    let display_name = obj
        .get_str("Name")
        .or_if_empty(|| obj.get_str("Name2"))
        .or_if_empty(|| obj.get_str("FullName"))
        .or_if_empty(|| obj.get_str("GetName"))
        .or_if_empty(|| obj.get_str("GetText"))
        .or_if_empty(|| com_type_hint.clone());
    let details = json!({
        "com_type_hint": com_type_hint,
    });
    (display_name, details)
}

trait StrFallbackExt {
    fn or_if_empty<F: FnOnce() -> String>(self, f: F) -> String;
}

impl StrFallbackExt for String {
    fn or_if_empty<F: FnOnce() -> String>(self, f: F) -> String {
        if self.trim().is_empty() {
            f()
        } else {
            self
        }
    }
}

#[derive(Default)]
struct DocStageTimings {
    bbox_ms: u64,
    custom_props_ms: u64,
    whats_wrong_ms: u64,
}

// =============================================================================
// Layer 3: O(1) Document-Level Context (FR-3)
// =============================================================================
#[allow(clippy::too_many_arguments)]
fn collect_doc_context(
    doc: &DispObj,
    ext: Option<&DispObj>,
    doc_title: String,
    doc_path: String,
    doc_type: String,
    doc_type_num: i32,
    equation_count: usize,
    skip_flags: crate::pick::adapters::solidworks::AdaptiveSkipFlags,
    hard_deadline: Instant,
) -> (SwDocContext, DocStageTimings) {
    let mut timings = DocStageTimings::default();

    let active_config = doc
        .get_disp("ConfigurationManager")
        .and_then(|cm| cm.get_disp("ActiveConfiguration"))
        .map(|ac| ac.get_str("Name"))
        .unwrap_or_default();

    let config_names = doc
        .call("GetConfigurationNames", Vec::new())
        .ok()
        .and_then(|v| v.as_string_vec())
        .map(|list| list.into_iter().take(SPEC_MAX_CONFIG_NAMES).collect())
        .unwrap_or_default();

    let is_dirty = doc.get_bool("GetSaveFlag").unwrap_or(false);

    // Edit state: active sketch + assembly in-context component edit
    let active_sketch_name = doc
        .get_disp("SketchManager")
        .and_then(|sm| sm.get_disp("ActiveSketch"))
        .map(|sk| {
            sk.get_str("Name")
                .or_if_empty(|| sk.get_disp("GetFeature").map(|f| f.get_str("Name")).unwrap_or_default())
        })
        .unwrap_or_default();
    let in_sketch_edit = !active_sketch_name.is_empty();

    let editing_component = if doc_type_num == 2 {
        doc.call("GetEditTarget2", Vec::new())
            .ok()
            .and_then(|v| v.as_dispatch())
            .map(|c| c.get_str("Name2").or_if_empty(|| c.get_str("GetTitle")))
            .filter(|n| !n.eq_ignore_ascii_case(&doc_title))
            .unwrap_or_default()
    } else {
        String::new()
    };
    let in_component_edit = !editing_component.is_empty();

    // Document units
    let len_pref = doc
        .call(
            "GetUserPreferenceIntegerValue",
            vec![VariantGuard::from_i32(SW_UNITS_LINEAR)],
        )
        .ok()
        .and_then(|v| v.as_i32())
        .unwrap_or(0);
    let length_unit = match len_pref {
        0 => "mm",
        1 => "cm",
        2 => "m",
        3..=5 => "in",
        _ => "mm",
    }
    .to_string();
    let ang_pref = doc
        .call(
            "GetUserPreferenceIntegerValue",
            vec![VariantGuard::from_i32(SW_UNITS_ANGULAR)],
        )
        .ok()
        .and_then(|v| v.as_i32())
        .unwrap_or(0);
    let angle_unit = match ang_pref {
        2 => "rad",
        _ => "deg",
    }
    .to_string();

    // Solid / surface body counts (Part docs: O(1) GetBodies2)
    let (solid_body_count, surface_body_count) = if doc_type_num == 1 {
        let sc = doc
            .call(
                "GetBodies2",
                vec![VariantGuard::from_i32(0), VariantGuard::from_bool(false)],
            )
            .ok()
            .and_then(|v| v.as_dispatch_vec())
            .map(|v| v.len())
            .unwrap_or(0);
        let sfc = doc
            .call(
                "GetBodies2",
                vec![VariantGuard::from_i32(1), VariantGuard::from_bool(false)],
            )
            .ok()
            .and_then(|v| v.as_dispatch_vec())
            .map(|v| v.len())
            .unwrap_or(0);
        (sc, sfc)
    } else {
        (0, 0)
    };

    // Overall model bounding box (in mm), guarded by SwAdaptiveProfiler on huge assemblies
    let bbox_mm = if !skip_flags.skip_bbox && Instant::now() < hard_deadline {
        let t0 = Instant::now();
        let raw = match doc_type_num {
            1 => doc
                .call("GetPartBox", vec![VariantGuard::from_bool(true)])
                .ok()
                .and_then(|v| v.as_f64_vec()),
            2 => doc
                .call("GetBox", vec![VariantGuard::from_i32(0)])
                .ok()
                .and_then(|v| v.as_f64_vec()),
            _ => None,
        };
        timings.bbox_ms = t0.elapsed().as_millis() as u64;
        raw.filter(|b| b.len() >= 6).map(|b| {
            [
                round_dec(b[0] * 1000.0, 3),
                round_dec(b[1] * 1000.0, 3),
                round_dec(b[2] * 1000.0, 3),
                round_dec(b[3] * 1000.0, 3),
                round_dec(b[4] * 1000.0, 3),
                round_dec(b[5] * 1000.0, 3),
            ]
        })
    } else {
        None
    };

    // O(1) Error/Warning features via IModelDocExtension::GetWhatsWrongCount + GetWhatsWrong
    // Zero feature-tree iteration, zero magic cutoff!
    let (error_feature_count, error_features) =
        if !skip_flags.skip_whats_wrong && Instant::now() < hard_deadline {
            let t0 = Instant::now();
            let res = ext
                .map(collect_whats_wrong_o1)
                .unwrap_or((0, Vec::new()));
            timings.whats_wrong_ms = t0.elapsed().as_millis() as u64;
            res
        } else {
            (0, Vec::new())
        };

    // Custom properties (active configuration merged over document-level, up to 20 non-empty)
    let custom_properties =
        if !skip_flags.skip_custom_props && Instant::now() < hard_deadline {
            let t0 = Instant::now();
            let props = ext
                .map(|e| collect_custom_properties(e, &active_config))
                .unwrap_or_default();
            timings.custom_props_ms = t0.elapsed().as_millis() as u64;
            props
        } else {
            BTreeMap::new()
        };

    let ctx = SwDocContext {
        doc: SwDocInfo {
            title: doc_title,
            path: doc_path,
            doc_type,
            active_config,
            config_names,
            is_dirty,
        },
        edit_state: SwEditState {
            in_sketch_edit,
            active_sketch_name,
            in_component_edit,
            editing_component,
        },
        units: SwUnits {
            length: length_unit,
            angle: angle_unit,
        },
        model_summary: SwModelSummary {
            solid_body_count,
            surface_body_count,
            bbox_mm,
            equation_count,
            error_feature_count,
            error_features,
        },
        custom_properties,
    };

    (ctx, timings)
}

fn collect_whats_wrong_o1(ext: &DispObj) -> (usize, Vec<String>) {
    let count = ext.get_i32("GetWhatsWrongCount").unwrap_or(0).max(0) as usize;
    if count == 0 {
        return (0, Vec::new());
    }
    let mut features_var = VARIANT::default();
    let mut codes_var = VARIANT::default();
    let mut warns_var = VARIANT::default();
    let ok = ext
        .call(
            "GetWhatsWrong",
            vec![
                VariantGuard::byref_variant(&mut features_var),
                VariantGuard::byref_variant(&mut codes_var),
                VariantGuard::byref_variant(&mut warns_var),
            ],
        )
        .ok()
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let f_guard = VariantGuard(features_var);
    let _c_guard = VariantGuard(codes_var);
    let _w_guard = VariantGuard(warns_var);

    if !ok {
        return (count, Vec::new());
    }
    let names = f_guard
        .as_dispatch_vec()
        .map(|feats| {
            feats
                .into_iter()
                .take(SPEC_MAX_ERROR_FEATURES)
                .map(|f| f.get_str("Name"))
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    (count, names)
}

fn collect_custom_properties(ext: &DispObj, active_config: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    // Read document-level first, then active configuration overrides
    let scopes: Vec<&str> = if active_config.is_empty() {
        vec![""]
    } else {
        vec!["", active_config]
    };

    for scope in scopes {
        let Some(mgr) = ext
            .get_with("CustomPropertyManager", vec![VariantGuard::from_str(scope)])
            .ok()
            .and_then(|v| v.as_dispatch())
        else {
            continue;
        };
        let mut names_v = VARIANT::default();
        let mut types_v = VARIANT::default();
        let mut vals_v = VARIANT::default();
        let mut resolved_v = VARIANT::default();
        let mut linked_v = VARIANT::default();
        let _ = mgr.call(
            "GetAll3",
            vec![
                VariantGuard::byref_variant(&mut names_v),
                VariantGuard::byref_variant(&mut types_v),
                VariantGuard::byref_variant(&mut vals_v),
                VariantGuard::byref_variant(&mut resolved_v),
                VariantGuard::byref_variant(&mut linked_v),
            ],
        );
        let g_names = VariantGuard(names_v);
        let _g_types = VariantGuard(types_v);
        let g_vals = VariantGuard(vals_v);
        let g_resolved = VariantGuard(resolved_v);
        let _g_linked = VariantGuard(linked_v);

        let names = g_names.as_string_vec().unwrap_or_default();
        let vals = g_vals.as_string_vec().unwrap_or_default();
        let resolved = g_resolved.as_string_vec().unwrap_or_default();

        for (i, k) in names.into_iter().enumerate() {
            let key = k.trim().to_string();
            if key.is_empty() {
                continue;
            }
            let val = resolved
                .get(i)
                .filter(|s| !s.trim().is_empty())
                .or_else(|| vals.get(i))
                .map(|s| s.trim().to_string())
                .unwrap_or_default();
            if val.is_empty() {
                continue;
            }
            if out.contains_key(&key) || out.len() < SPEC_MAX_CUSTOM_PROPERTIES {
                out.insert(key, val);
            }
        }
    }
    out
}
