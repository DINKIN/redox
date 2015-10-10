//To use this, please install zfs-fuse
use redox::*;

pub mod nvpair;
pub mod nvstream;
pub mod xdr;

#[repr(packed)]
pub struct VdevLabel {
    pub blank: [u8; 8 * 1024],
    pub boot_header: [u8; 8 * 1024],
    pub nv_pairs: [u8; 112 * 1024],
    pub uberblocks: [Uberblock; 128],
}

impl VdevLabel {
    pub fn from(data: &[u8]) -> Option<Self> {
        if data.len() >= mem::size_of::<VdevLabel>() {
            let vdev_label = unsafe { ptr::read(data.as_ptr() as *const VdevLabel) };
            Some(vdev_label)
        } else {
            Option::None
        }
    }
}

#[derive(Copy, Clone, Debug)]
#[repr(packed)]
pub struct Uberblock {
    pub magic: u64,
    pub version: u64,
    pub txg: u64,
    pub guid_sum: u64,
    pub timestamp: u64,
    pub rootbp: BlockPtr,
}

impl Uberblock {
    pub fn magic_little() -> u64 {
        return 0x0cb1ba00;
    }

    pub fn magic_big() -> u64 {
        return 0x00bab10c;
    }

    pub fn from(data: &Vec<u8>) -> Option<Self> {
        if data.len() >= mem::size_of::<Uberblock>() {
            let uberblock = unsafe { ptr::read(data.as_ptr() as *const Uberblock) };
            if uberblock.magic == Uberblock::magic_little() {
                return Option::Some(uberblock);
            } else if uberblock.magic == Uberblock::magic_big() {
                return Option::Some(uberblock);
            } else if uberblock.magic > 0 {
                println!("Unknown Magic: {:X}", uberblock.magic as usize);
            }
        }

        Option::None
    }
}

#[derive(Copy, Clone)]
#[repr(packed)]
pub struct DVAddr {
    pub vdev: u64,
    pub offset: u64,
}

impl DVAddr {
    /// Sector address is the offset plus two vdev labels and one boot block (4 MB, or 8192 sectors)
    pub fn sector(&self) -> u64 {
        self.offset() + 0x2000
    }

    pub fn gang(&self) -> bool {
        if self.offset&0x8000000000000000 == 1 {
            true
        } else {
            false
        }
    }

    pub fn offset(&self) -> u64 {
        self.offset & 0x7FFFFFFFFFFFFFFF
    }

    pub fn asize(&self) -> u64 {
        (self.vdev & 0xFFFFFF) + 1
    }
}

impl fmt::Debug for DVAddr {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        try!(write!(f, "DVAddr {{ offset: {:X}, gang: {}, asize: {:X} }}\n",
                    self.offset(), self.gang(), self.asize()));
        Ok(())
    }
}

#[derive(Copy, Clone, Debug)]
#[repr(packed)]
pub struct BlockPtr {
    pub dvas: [DVAddr; 3],
    pub flags_size: u64,
    pub padding: [u64; 3],
    pub birth_txg: u64,
    pub fill_count: u64,
    pub checksum: [u64; 4],
}

impl BlockPtr {
    pub fn object_type(&self) -> u64 {
        (self.flags_size >> 48) & 0xFF
    }

    pub fn checksum(&self) -> u64 {
        (self.flags_size >> 40) & 0xFF
    }

    pub fn compression(&self) -> u64 {
        (self.flags_size >> 32) & 0xFF
    }

    pub fn lsize(&self) -> u64 {
        (self.flags_size) & 0xFFFF + 1
    }

    pub fn psize(&self) -> u64 {
        ((self.flags_size) >> 16) & 0xFFFF + 1
    }
}

#[derive(Copy, Clone, Debug)]
#[repr(packed)]
pub struct Gang {
    pub bps: [BlockPtr; 3],
    pub padding: [u64; 14],
    pub magic: u64,
    pub checksum: u64,
}

impl Gang {
    pub fn magic() -> u64 {
        return 0x117a0cb17ada1002;
    }
}

#[repr(packed)]
pub struct DNodePhys {
    pub object_type: u8,
    pub indblkshift: u8, // ln2(indirect block size)
    pub nlevels: u8, // 1=blkptr->data blocks
    pub nblkptr: u8, // length of blkptr
    pub bonus_type: u8, // type of data in bonus buffer
    pub checksum: u8, // ZIO_CHECKSUM type
    pub compress: u8, // ZIO_COMPRESS type
    pub flags: u8, // DNODE_FLAG_*
    pub data_blk_sz_sec: u16, // data block size in 512b sectors
    pub bonus_len: u16, // length of bonus
    pub pad2: [u8; 4],

    // accounting is protected by dirty_mtx
    pub maxblkid: u64, // largest allocated block ID
    pub used: u64, // bytes (or sectors) of disk space

    pub pad3: [u64; 4],

    blkptr_bonus: [u8; 448],
}

impl DNodePhys {
    pub fn from(data: &[u8]) -> Option<Self> {
        if data.len() >= mem::size_of::<DNodePhys>() {
            Some(unsafe { ptr::read(data.as_ptr() as *const DNodePhys) })
        } else {
            Option::None
        }
    }

    pub fn get_blkptr<'a>(&self, i: usize) -> &'a BlockPtr {
        unsafe { mem::transmute(&self.blkptr_bonus[i*128]) }
    }

    pub fn get_bonus(&self) -> &[u8] {
        &self.blkptr_bonus[(self.nblkptr as usize)*128..]
    }
}

impl fmt::Debug for DNodePhys {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        try!(write!(f, "DNodePhys {{ object_type: {:X}, nlevels: {:X}, nblkptr: {:X}, bonus_type: {:X}, bonus_len: {:X}}}\n",
                    self.object_type, self.nlevels, self.nblkptr, self.bonus_type, self.bonus_len));
        Ok(())
    }
}

#[repr(packed)]
pub struct ObjectSetPhys {
    pad: u8,
    meta_dnode: DNodePhys,
    zil_header: ZilHeader,
    os_type: u64,
    //pad: [u8; 360],
}

impl ObjectSetPhys {
    pub fn from(data: &[u8]) -> Option<Self> {
        if data.len() >= mem::size_of::<ObjectSetPhys>() {
            Some(unsafe { ptr::read(data.as_ptr() as *const ObjectSetPhys) })
        } else {
            Option::None
        }
    }
}

#[repr(packed)]
pub struct ZilHeader {
    claim_txg: u64,
    replay_seq: u64,
    log: BlockPtr,
}

pub struct ZFS {
    disk: File,
}

impl ZFS {
    pub fn new(disk: File) -> Self {
        ZFS { disk: disk }
    }

    //TODO: Error handling
    pub fn read(&mut self, start: usize, length: usize) -> Vec<u8> {
        let mut ret: Vec<u8> = vec![0; length*512];

        self.disk.seek(Seek::Start(start * 512));
        self.disk.read(&mut ret);

        return ret;
    }

    pub fn write(&mut self, block: usize, data: &[u8; 512]) {
        self.disk.seek(Seek::Start(block * 512));
        self.disk.write(data);
    }

    pub fn uber(&mut self) -> Option<Uberblock> {
        let mut newest_uberblock: Option<Uberblock> = Option::None;
        for i in 0..128 {
            match Uberblock::from(&self.read(256 + i * 2, 2)) {
                Option::Some(uberblock) => {
                    let mut newest = false;
                    match newest_uberblock {
                        Option::Some(previous) => {
                            if uberblock.txg > previous.txg {
                                newest = true;
                            }
                        }
                        Option::None => newest = true,
                    }

                    if newest {
                        newest_uberblock = Option::Some(uberblock);
                    }
                }
                Option::None => (), //Invalid uberblock
            }
        }
        return newest_uberblock;
    }
}

//TODO: Find a way to remove all the to_string's
pub fn main() {
    console_title("ZFS");

    let red = [255, 127, 127, 255];
    let green = [127, 255, 127, 255];
    let blue = [127, 127, 255, 255];

    println!("Type open zfs.img to open the image file");

    let mut zfs_option: Option<ZFS> = Option::None;

    while let Option::Some(line) = readln!() {
        let mut args: Vec<String> = Vec::new();
        for arg in line.split(' ') {
            args.push(arg.to_string());
        }

        if let Option::Some(command) = args.get(0) {
            println!("# {}", line);

            let mut close = false;
            match zfs_option {
                Option::Some(ref mut zfs) => {
                    if *command == "uber".to_string() {
                        //128 KB of ubers after 128 KB of other stuff
                        match zfs.uber() {
                            Some(uberblock) => {
                                println_color!(green, "Newest Uberblock {:X}", uberblock.magic);
                                println!("Version {}", uberblock.version);
                                println!("TXG {}", uberblock.txg);
                                println!("GUID {:X}", uberblock.guid_sum);
                                println!("Timestamp {}", uberblock.timestamp);
                                println!("ROOTBP[0] {:?}", uberblock.rootbp.dvas[0]);
                                println!("ROOTBP[1] {:?}", uberblock.rootbp.dvas[1]);
                                println!("ROOTBP[2] {:?}", uberblock.rootbp.dvas[2]);
                            }
                            None => println_color!(red, "No valid uberblock found!"),
                        }
                    } else if *command == "vdev_label".to_string() {
                        match VdevLabel::from(&zfs.read(0, 256 * 2)) {
                            Some(ref mut vdev_label) => {
                                let mut xdr = xdr::MemOps::new(&mut vdev_label.nv_pairs);
                                let nv_list = nvstream::decode_nv_list(&mut xdr);
                                println_color!(green, "Got nv_list:\n{:?}", nv_list);
                            },
                            None => { println_color!(red, "Couldn't read vdev_label"); },
                        }
                    } else if *command == "mos".to_string() {
                        match zfs.uber() {
                            Some(uberblock) => {
                                let mos_dva = uberblock.rootbp.dvas[0];
                                println_color!(green, "DVA: {:?}", mos_dva);
                                println_color!(green, "type: {:X}", uberblock.rootbp.object_type());
                                println_color!(green, "checksum: {:X}", uberblock.rootbp.checksum());
                                println_color!(green, "compression: {:X}", uberblock.rootbp.compression());
                                println!("Reading {} sectors starting at {}", mos_dva.asize(), mos_dva.sector());
                                println!("ObjectSetPhys size: {}", mem::size_of::<ObjectSetPhys>());
                                println!("DNodePhys size: {}", mem::size_of::<DNodePhys>());
                                let mut mos = zfs.read(mos_dva.sector() as usize, mos_dva.asize() as usize);
                                let obj_set = ObjectSetPhys::from(&mos[..]);
                                if let Some(ref obj_set) = obj_set {
                                    println!("meta dnode: {:?}", obj_set.meta_dnode);
                                    println!("os_type: {:X}", obj_set.os_type);
                                }
                                /*let mut xdr = xdr::MemOps::new(&mut mos);
                                let nv_list = nvstream::decode_nv_list(&mut xdr);
                                println_color!(green, "Got nv_list:\n{:?}", nv_list);*/
                            },
                            None => println_color!(red, "No valid uberblock found!"),
                        }
                    } else if *command == "dump".to_string() {
                        match args.get(1) {
                            Some(arg) => {
                                let sector = arg.to_num();
                                println_color!(green, "Dump sector: {}", sector);

                                let data = zfs.read(sector, 1);
                                for i in 0..data.len() {
                                    if i % 32 == 0 {
                                        print!("\n{:X}:", i);
                                    }
                                    if let Option::Some(byte) = data.get(i) {
                                        print!(" {:X}", *byte);
                                    } else {
                                        println!(" !");
                                    }
                                }
                                print!("\n");
                            }
                            None => println_color!(red, "No sector specified!"),
                        }
                    } else if *command == "close".to_string() {
                        println_color!(red, "Closing");
                        close = true;
                    } else {
                        println_color!(blue, "Commands: uber vdev_label mos dump close");
                    }
                }
                Option::None => {
                    if *command == "open".to_string() {
                        match args.get(1) {
                            Option::Some(arg) => {
                                println_color!(green, "Open: {}", arg);
                                zfs_option = Option::Some(ZFS::new(File::open(arg)));
                            }
                            Option::None => println_color!(red, "No file specified!"),
                        }
                    } else {
                        println_color!(blue, "Commands: open");
                    }
                }
            }
            if close {
                zfs_option = Option::None;
            }
        }
    }
}
