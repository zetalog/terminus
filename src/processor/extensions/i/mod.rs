use terminus_global::*;
use std::rc::Rc;
use crate::processor::extensions::{HasCsr, NoStepCb};

mod insns;
pub mod csrs;

use csrs::ICsrs;
use crate::processor::{PrivilegeLevel, Privilege, ProcessorState};

pub struct ExtensionI {
    csrs: Rc<ICsrs>,
}

impl ExtensionI {
    pub fn new(state: &ProcessorState) -> ExtensionI {
        let cfg = state.config();
        let e = ExtensionI {
            csrs: Rc::new(ICsrs::new(cfg.xlen))
        };
        //no debug
        e.csrs.tselect_mut().set(0xffff_ffff_ffff_ffff);
        //mstatus
        //sd bit
        e.csrs.mstatus_mut().sd_transform({
            let csrs = e.csrs.clone();
            move |_| {
                if csrs.mstatus().fs() == 0x3 && csrs.mstatus().xs() == 0x3 {
                    1
                } else {
                    0
                }
            }
        }
        );
        //will be overrided if 's' implemented
        e.csrs.mstatus_mut().set_tvm_transform(|_| { 0 });
        e.csrs.mstatus_mut().set_tsr_transform(|_| { 0 });

        //privilege_level config
        match cfg.privilege_level() {
            PrivilegeLevel::MSU => {}
            PrivilegeLevel::MU => {
                e.csrs.mstatus_mut().set_mpp_transform(|mpp| {
                    if mpp != 0 {
                        let m: u8 = Privilege::M.into();
                        m as RegT
                    } else {
                        0
                    }
                });
            }
            PrivilegeLevel::M => {
                let m: u8 = Privilege::M.into();
                e.csrs.mstatus_mut().set_mpp(m as RegT);
                e.csrs.mstatus_mut().set_mpp_transform(move |_| {
                    m as RegT
                });
                e.csrs.mstatus_mut().set_tw_transform(|_| { 0 });
            }
        }

        //deleg counter
        e.csrs.instret_mut().instret_transform({
            let count = state.insns_cnt().clone();
            move |_| {
                *count.borrow() as RegT
            }
        }
        );
        e.csrs.instreth_mut().instret_transform({
            let count = state.insns_cnt().clone();
            move |_| {
                (*count.borrow() >> 32) as RegT
            }
        }
        );
        e.csrs.minstret_mut().instret_transform({
            let count = state.insns_cnt().clone();
            move |_| {
                *count.borrow() as RegT
            }
        }
        );
        e.csrs.minstreth_mut().instret_transform({
            let count = state.insns_cnt().clone();
            move |_| {
                (*count.borrow() >> 32) as RegT
            }
        }
        );
        e
    }

    pub fn get_csrs(&self) -> &Rc<ICsrs> {
        &self.csrs
    }
}

impl HasCsr for ExtensionI {
    fn csr_write(&self, state: &ProcessorState, addr: InsnT, value: RegT) -> Option<()> {
        if value & ((1 as RegT) << (('c' as u8 - 'a' as u8) as RegT)) == 0 && addr == 0x301 && state.pc().trailing_zeros() == 1 {
            return Some(())
        }
        self.csrs.write(addr, value)
    }
    fn csr_read(&self, state: &ProcessorState, addr: InsnT) -> Option<RegT> {
        let addr_high = addr & 0xff0;
        if (addr_high == 0xc80 || addr_high == 0xc90 || addr_high == 0xb80 || addr_high == 0xb90) && state.config().xlen != XLen::X32 {
            return None
        }
        if addr_high == 0xc80 || addr_high == 0xc90 || addr_high == 0xc00 || addr_high == 0xc10 {
            match state.privilege() {
                Privilege::M => {}
                Privilege::S => {
                    if self.csrs.mcounteren().get() & ((1 as RegT) << (addr as RegT & 0x1f)) == 0 {
                        return None
                    }
                }
                Privilege::U => {
                    if self.csrs.mcounteren().get() & ((1 as RegT) << (addr as RegT & 0x1f)) == 0 {
                        return None
                    }
                    if state.check_extension('s').is_ok()  {
                        if state.scsrs().scounteren().get() & ((1 as RegT) << (addr as RegT & 0x1f)) == 0 {
                            return None
                        }
                    }
                }
            }
        }
        self.csrs.read(addr)
    }
}

impl NoStepCb for ExtensionI{}

