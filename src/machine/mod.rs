use terminus_spaceport::memory::{GHEAP, Region, BytesAccess, MemInfo};
use terminus_spaceport::space::{Space, SPACE_TABLE};
use terminus_spaceport::space;
use std::sync::Arc;
use std::{fs, fmt};
use super::elf::ElfLoader;
use std::ops::Deref;
use super::devices::htif::HTIF;
use std::fmt::{Display, Formatter};

pub struct Machine {
    name: String,
    mem_space: Arc<Space>,
}

impl Machine {
    pub fn new(name: &str) -> Machine {
        Machine {
            name: name.to_string(),
            mem_space: SPACE_TABLE.get_space(name),
        }
    }

    fn register_region(&self, name: &str, base: u64, region: &Arc<Region>) -> Result<Arc<Region>, space::Error> {
        self.mem_space.add_region(name, &Region::remap(base, &region))
    }

    pub fn try_register_htif(&self, elf: &ElfLoader) {
        if let Some(s) = elf.htif_section() {
            self.register_region("htif", s.address(), &Region::io(0, 0x1000, Box::new(HTIF::new()))).unwrap();
        }
    }

    pub fn register_memory(&self, name: &str, base: u64, mem: &Arc<Region>) {
        match self.register_region(name, base, &mem) {
            Ok(_) => {}
            Err(space::Error::Overlap(n, msg)) => {
                if n == "htif".to_string() {
                    let htif_region = self.mem_space.get_region(&n).unwrap();
                    let range0 = if base < htif_region.info.base {
                        Some(MemInfo { base: base, size: htif_region.info.base - base })
                    } else {
                        None
                    };
                    let range1 = if base + mem.info.size > htif_region.info.base + htif_region.info.size {
                        Some(MemInfo { base: htif_region.info.base + htif_region.info.size, size: base + mem.info.size - (htif_region.info.base + htif_region.info.size) })
                    } else {
                        None
                    };
                    range0.iter().for_each(|info| {
                        self.register_region(&format!("{}_0", name), info.base, &Region::remap_partial(0, mem, 0, info.size)).unwrap();
                    });
                    range1.iter().for_each(|info| {
                        self.register_region(&format!("{}_1", name), info.base, &Region::remap_partial(0, mem, info.base - base, info.size)).unwrap();
                    });
                } else {
                    panic!(msg)
                }
            }
            Err(space::Error::Renamed(_, msg)) => panic!(msg)
        }
    }

    pub fn load_elf(&self, elf: &ElfLoader) {
        elf.load(|addr, data| {
            let region = self.mem_space.get_region_by_addr(addr).unwrap();
            if addr + data.len() as u64 > region.info.base + region.info.size {
                Err(format!("not enough memory!"))
            } else {
                Ok(BytesAccess::write(region.deref(), addr, data))
            }
        }).expect(&format!("{} load elf fail!", self.name));
    }
}

impl Display for Machine {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        writeln!(f, "Machine {}:", self.name)?;
        writeln!(f, "   {}", self.mem_space.to_string())
    }
}

#[test]
fn machine_basic() {
    let m = Machine::new("m0");
    let blob = fs::read("top_tests/elf/rv64ui-p-add").expect("Can't read binary");
    let elf = ElfLoader::new(blob.as_slice()).expect("Invalid ELF {}!");
    m.try_register_htif(&elf);
    m.register_memory("main_memory", 0x80000000, &GHEAP.alloc(0x10000000, 1).expect("main_memory alloc fail!"));
    m.register_memory("rom", 0x20000000, &GHEAP.alloc(0x10000000, 1).expect("rom alloc fail!"));
    m.load_elf(&elf);
    println!("{}", m.to_string())
}

