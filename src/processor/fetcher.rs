use crate::processor::ProcessorState;
use crate::processor::insn::Instruction;
use std::rc::Rc;
use terminus_global::{RegT, InsnT};
use crate::processor::mmu::{Mmu, MmuOpt};
use crate::processor::trap::Exception;
use crate::processor::decode::*;
use std::sync::Arc;
use crate::devices::bus::Bus;
use std::cell::RefCell;
use std::mem::MaybeUninit;

struct ICacheEntry {
    accessed: bool,
    tag: u64,
    insn: Option<(InsnT, &'static Instruction)>,
}

struct ICacheBasket {
    ptr: u8,
    entries: [ICacheEntry; 16],
}

impl ICacheBasket {
    fn new() -> ICacheBasket {
        ICacheBasket {
            ptr: 0,
            entries: unsafe {
                let mut arr: MaybeUninit<[ICacheEntry; 16]> = MaybeUninit::uninit();
                for i in 0..16 {
                    (arr.as_mut_ptr() as *mut ICacheEntry).add(i).write(ICacheEntry { accessed: false, tag: 0, insn: None });
                }
                arr.assume_init()
            },
        }
    }

    fn get_insn(&mut self, tag: u64) -> Option<(InsnT, &'static Instruction)> {
        let mut ptr = self.ptr;
        let tail = self.tail();
        while ptr != tail {
            let e = unsafe{self.entries.get_unchecked_mut(ptr as usize)};
            if e.tag == tag {
                if let Some(i) = e.insn {
                    e.accessed = true;
                    self.ptr = ptr;
                    return Some(i);
                }
            }
            e.accessed = false;
            ptr = Self::next_ptr(ptr);
        }
        None
    }

    fn next_ptr(p: u8) -> u8 {
        if p == 15 {
            0
        } else {
            p + 1
        }
    }

    fn prev_ptr(p: u8) -> u8 {
        if p == 0 {
            15
        } else {
            p - 1
        }
    }

    fn tail(&self) -> u8 {
        if self.ptr == 0 {
            15
        } else {
            self.ptr - 1
        }
    }

    fn set_entry(&mut self, tag: u64, ir:InsnT, insn: &'static Instruction) {
        let mut ptr = self.tail();
        let tail = self.ptr;
        while ptr != tail {
            let e = unsafe{self.entries.get_unchecked(ptr as usize)};
            if e.insn.is_none() || !e.accessed {
                break;
            }
            ptr = Self::prev_ptr(ptr);
        }
        let e = unsafe{self.entries.get_unchecked_mut(ptr as usize)};
        e.accessed = true;
        e.tag = tag;
        e.insn = Some((ir, insn));
        self.ptr = ptr;
    }

    fn invalid_all(&mut self) {
        self.entries.iter_mut().for_each(|e| { e.insn = None })
    }
}


struct ICache {
    size: usize,
    baskets: Vec<ICacheBasket>,
}

impl ICache {
    fn new(size: usize) -> ICache {
        let mut cache = ICache {
            size,
            baskets: Vec::with_capacity(size),
        };
        for _ in 0..size {
            cache.baskets.push(ICacheBasket::new())
        };
        cache
    }
    #[cfg_attr(feature = "no-inline", inline(never))]
    fn get_insn(&mut self, addr: u64) -> Option<(InsnT, &'static Instruction)> {
        unsafe {self.baskets.get_unchecked_mut(((addr >> 1) as usize) & (self.size - 1))}.get_insn(addr >> 1)
    }
    #[cfg_attr(feature = "no-inline", inline(never))]
    fn set_entry(&mut self, addr: u64, ir:InsnT, insn: &'static Instruction) {
        unsafe {self.baskets.get_unchecked_mut(((addr >> 1) as usize) & (self.size - 1))}.set_entry(addr >> 1, ir, insn)
    }

    fn invalid_all(&mut self) {
        self.baskets.iter_mut().for_each(|b| { b.invalid_all() })
    }
}

pub struct Fetcher {
    p: Rc<ProcessorState>,
    bus: Arc<Bus>,
    icache: RefCell<ICache>,
}

impl Fetcher {
    pub fn new(p: &Rc<ProcessorState>, bus: &Arc<Bus>) -> Fetcher {
        Fetcher {
            p: p.clone(),
            bus: bus.clone(),
            icache: RefCell::new(ICache::new(1024)),
        }
    }
    #[cfg_attr(feature = "no-inline", inline(never))]
    fn fetch_u16_slow(&self, addr: u64, pc: u64) -> Result<InsnT, Exception> {
        match self.bus.read_u16(addr) {
            Ok(data) => {
                Ok(data as InsnT)
            }
            Err(_) => Err(Exception::FetchAccess(pc)),
        }
    }
    #[cfg_attr(feature = "no-inline", inline(never))]
    fn fetch_u32_slow(&self, addr: u64, pc: u64) -> Result<InsnT, Exception> {
        match self.bus.read_u32(addr) {
            Ok(data) => Ok(data as InsnT),
            Err(_) => Err(Exception::FetchAccess(pc))
        }
    }

    pub fn flush_icache(&self) {
        self.icache.borrow_mut().invalid_all()
    }

    pub fn fetch(&self, pc: RegT, mmu: &Mmu) -> Result<&'static Instruction, Exception> {
        let mut icache = self.icache.borrow_mut();
        if pc.trailing_zeros() == 1 {
            let pa = mmu.translate(pc, 2, MmuOpt::Fetch)?;
            if let Some((ir, ref insn)) = icache.get_insn(pa) {
                self.p.set_ir(ir);
                Ok(insn)
            } else {
                let data_low = self.fetch_u16_slow(pa, pc)?;
                if data_low & 0x3 != 0x3 {
                    let data = data_low as u16 as InsnT;
                    let insn = GDECODER.decode(data)?;
                    icache.set_entry(pa, data, insn);
                    self.p.set_ir(data);
                    Ok(insn)
                } else {
                    let pa_high = mmu.translate(pc + 2, 2, MmuOpt::Fetch)?;
                    let data_high = self.fetch_u16_slow(pa_high, pc)?;
                    let data = data_low as u16 as InsnT | ((data_high as u16 as InsnT) << 16);
                    let insn = GDECODER.decode(data)?;
                    icache.set_entry(pa, data, insn);
                    self.p.set_ir(data);
                    Ok(insn)
                }
            }
        } else {
            let pa = mmu.translate(pc, 4, MmuOpt::Fetch)?;
            if let Some((ir, insn)) = icache.get_insn(pa) {
                self.p.set_ir(ir);
                Ok(insn)
            } else {
                let data = self.fetch_u32_slow(pa, pc)?;
                if data & 0x3 != 0x3 {
                    let data_low = data as u16 as InsnT;
                    let insn = GDECODER.decode(data_low)?;
                    icache.set_entry(pa, data, insn);
                    self.p.set_ir(data);
                    Ok(insn)
                } else {
                    let insn = GDECODER.decode(data)?;
                    icache.set_entry(pa, data, insn);
                    self.p.set_ir(data);
                    Ok(insn)
                }
            }
        }
    }
}
