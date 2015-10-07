use alloc::arc::Arc;
use alloc::boxed::Box;

use core::{cmp, mem, ptr};
use core::sync::atomic::{AtomicBool, Ordering};

use drivers::disk::*;
use drivers::pio::*;
use drivers::pciconfig::PCIConfig;

use common::debug;
use common::queue::Queue;
use common::memory;
use common::memory::Memory;
use common::resource::{NoneResource, Resource, ResourceSeek, ResourceType, URL, VecResource};
use common::scheduler::*;
use common::string::{String, ToString};
use common::vec::Vec;

use programs::common::SessionItem;

use syscall::call::sys_yield;

#[repr(packed)]
pub struct Header {
    pub signature: [u8; 8],
    pub version: u32,
    pub name: [u8; 244],
    pub extents: [Extent; 16],
}

#[repr(packed)]
pub struct NodeData {
    pub name: [u8; 256],
    pub extents: [Extent; 16],
}

pub struct Node {
    pub address: u64,
    pub name: String,
    pub extents: [Extent; 16],
}

impl Node {
    pub fn new(address: u64, data: &NodeData) -> Self {
        let mut utf8: Vec<u8> = Vec::new();
        for i in 0..data.name.len() {
            let c = data.name[i];
            if c == 0 {
                break;
            } else {
                utf8.push(c);
            }
        }

        Node {
            address: address,
            name: String::from_utf8(&utf8),
            extents: data.extents,
        }
    }
}

impl Clone for Node {
    fn clone(&self) -> Self {
        Node {
            address: self.address,
            name: self.name.clone(),
            extents: self.extents,
        }
    }
}

pub struct FileSystem {
    pub disk: Disk,
    pub header: Header,
    pub nodes: Vec<Node>,
}

impl FileSystem {
    pub fn from_disk(mut disk: Disk) -> Option<Self> {
        unsafe {
            if disk.identify() {
                debug::d(" Disk Found");

                let header_ptr: *const Header = memory::alloc_type();
                disk.read(1, 1, header_ptr as usize);
                let header = ptr::read(header_ptr);
                memory::unalloc(header_ptr as usize);

                if header.signature[0] == 'R' as u8 &&
                    header.signature[1] == 'E' as u8 &&
                    header.signature[2] == 'D' as u8 &&
                    header.signature[3] == 'O' as u8 &&
                    header.signature[4] == 'X' as u8 &&
                    header.signature[5] == 'F' as u8 &&
                    header.signature[6] == 'S' as u8 &&
                    header.signature[7] == '\0' as u8 &&
                    header.version == 0xFFFFFFFF {
                        debug::d(" Redox Filesystem\n");

                        let mut nodes = Vec::new();
                        for extent in &header.extents {
                            if extent.block > 0 && extent.length > 0 {
                                let mut node_data: Vec<NodeData> = Vec::new();
                                unsafe {
                                    let data = memory::alloc(extent.length as usize);
                                    if data > 0 {
                                        let sectors = (extent.length as usize + 511) / 512;
                                        let mut sector: usize = 0;
                                        while sectors - sector >= 65536 {
                                            let request = Request {
                                                extent: Extent {
                                                    block: extent.block + sector as u64,
                                                    length: 65536 * 512,
                                                },
                                                mem: data + sector * 512,
                                                read: true,
                                                complete: Arc::new(AtomicBool::new(false)),
                                            };

                                            disk.read(extent.block + sector as u64, 0, data + sector * 512);

                                            /*
                                            disk.request(request.clone());

                                            while request.complete.load(Ordering::SeqCst) == false {
                                                disk.on_poll();
                                            }
                                            */

                                            sector += 65535;
                                        }
                                        if sector < sectors {
                                            let request = Request {
                                                extent: Extent {
                                                    block: extent.block + sector as u64,
                                                    length: (sectors - sector) as u64 * 512,
                                                },
                                                mem: data + sector * 512,
                                                read: true,
                                                complete: Arc::new(AtomicBool::new(false)),
                                            };

                                            disk.read(extent.block + sector as u64, (sectors - sector) as u16, data + sector * 512);

                                            /*
                                            disk.request(request.clone());

                                            while request.complete.load(Ordering::SeqCst) == false {
                                                disk.on_poll();
                                            }
                                            */
                                        }

                                        node_data = Vec {
                                            data: data as *mut NodeData,
                                            length: extent.length as usize/mem::size_of::<NodeData>(),
                                        };
                                    }
                                }

                                for i in 0..node_data.len() {
                                    if let Some(data) = node_data.get(i) {
                                        nodes.push(Node::new(extent.block + i as u64, data));
                                    }
                                }
                            }
                        }

                        return Some(FileSystem {
                            disk: disk,
                            header: header,
                            nodes: nodes,
                        });
                } else {
                    debug::d(" Unknown Filesystem\n");
                }
            } else {
                debug::d(" Disk Not Found\n");
            }
        }

        Option::None
    }

    pub fn node(&self, filename: &String) -> Option<Node> {
        for node in self.nodes.iter() {
            if node.name == *filename {
                return Option::Some(node.clone());
            }
        }

        return Option::None;
    }

    pub fn list(&self, directory: &String) -> Vec<String> {
        let mut ret = Vec::<String>::new();

        for node in self.nodes.iter() {
            if node.name.starts_with(directory.clone()) {
                ret.push(node.name.substr(directory.len(), node.name.len() - directory.len()));
            }
        }

        return ret;
    }
}

pub struct FileResource {
    pub scheme: *mut FileScheme,
    pub node: Node,
    pub vec: Vec<u8>,
    pub seek: usize,
    pub dirty: bool,
}

impl Resource for FileResource {
    fn url(&self) -> URL {
        return URL::from_string(&("file:///".to_string() + &self.node.name));
    }

    fn stat(&self) -> ResourceType {
        return ResourceType::File;
    }

    fn read(&mut self, buf: &mut [u8]) -> Option<usize> {
        let mut i = 0;
        while i < buf.len() && self.seek < self.vec.len() {
            match self.vec.get(self.seek) {
                Option::Some(b) => buf[i] = *b,
                Option::None => (),
            }
            self.seek += 1;
            i += 1;
        }
        return Option::Some(i);
    }

    fn write(&mut self, buf: &[u8]) -> Option<usize> {
        let mut i = 0;
        while i < buf.len() && self.seek < self.vec.len() {
            self.vec.set(self.seek, buf[i]);
            self.seek += 1;
            i += 1;
        }
        while i < buf.len() {
            self.vec.push(buf[i]);
            self.seek += 1;
            i += 1;
        }
        if i > 0 {
            self.dirty = true;
        }
        return Option::Some(i);
    }

    fn seek(&mut self, pos: ResourceSeek) -> Option<usize> {
        match pos {
            ResourceSeek::Start(offset) => self.seek = offset,
            ResourceSeek::Current(offset) =>
                self.seek = cmp::max(0, self.seek as isize + offset) as usize,
            ResourceSeek::End(offset) =>
                self.seek = cmp::max(0, self.vec.len() as isize + offset) as usize,
        }
        while self.vec.len() < self.seek {
            self.vec.push(0);
        }
        return Option::Some(self.seek);
    }

    // TODO: Rename to sync
    // TODO: Check to make sure proper amount of bytes written. See Disk::write
    // TODO: Allow reallocation
    fn sync(&mut self) -> bool {
        if self.dirty {
            let block_size: usize = 512;

            let mut node_dirty = false;
            let mut pos: isize = 0;
            let mut remaining = self.vec.len() as isize;
            for ref mut extent in &mut self.node.extents {
                //Make sure it is a valid extent
                if extent.block > 0 && extent.length > 0 {
                    let current_sectors = (extent.length as usize + block_size - 1) / block_size;
                    let max_size = current_sectors * 512;

                    let size = cmp::min(remaining as usize, max_size);
                    let sectors = (size + block_size - 1) / block_size;

                    if size as u64 != extent.length {
                        extent.length = size as u64;
                        node_dirty = true;
                    }

                    unsafe {
                        let data = self.vec.as_ptr().offset(pos) as usize;
                        //TODO: Make sure data is copied safely into an zeroed area of the right size!

                        let reenable = start_no_ints();

                        let mut sector: usize = 0;
                        while sectors - sector >= 65536 {
                            (*self.scheme).fs.disk.write(extent.block + sector as u64,
                                            65535,
                                            data + sector * 512);
                            sector += 65535;
                        }
                        if sector < sectors {
                            (*self.scheme).fs.disk.write(extent.block + sector as u64,
                                            (sectors - sector) as u16,
                                            data + sector * 512);
                        }

                        end_no_ints(reenable);
                    }

                    pos += size as isize;
                    remaining -= size as isize;
                }
            }

            if node_dirty {
                debug::d("Node dirty, should rewrite\n");
            }

            self.dirty = false;

            if remaining > 0 {
                debug::d("Need to reallocate file, extra: ");
                debug::ds(remaining);
                debug::dl();
                return false;
            }
        }
        return true;
    }
}

impl Drop for FileResource {
    fn drop(&mut self) {
        self.sync();
    }
}

pub struct FileScheme {
    pci: PCIConfig,
    fs: FileSystem,
}

impl FileScheme {
    ///TODO Allow busmaster for secondary
    pub fn new(mut pci: PCIConfig) -> Option<Box<Self>> {
        unsafe { pci.flag(4, 4, true) }; // Bus mastering

        let base = unsafe { pci.read(0x20) } as u16 & 0xFFF0;

        debug::d("IDE on ");
        debug::dh(base as usize);
        debug::dl();

        debug::d("Primary Master:");
        if let Some(fs) = FileSystem::from_disk(Disk::primary_master(base)) {
            return Some(box FileScheme {
                pci: pci,
                fs: fs,
            });
        }

        debug::d("Primary Slave:");
        if let Some(fs) = FileSystem::from_disk(Disk::primary_slave(base)) {
            return Some(box FileScheme {
                pci: pci,
                fs: fs,
            });
        }

        debug::d("Secondary Master:");
        if let Some(fs) = FileSystem::from_disk(Disk::secondary_master(base)) {
            return Some(box FileScheme {
                pci: pci,
                fs: fs,
            });
        }

        debug::d("Secondary Slave:");
        if let Some(fs) = FileSystem::from_disk(Disk::secondary_slave(base)) {
            return Some(box FileScheme {
                pci: pci,
                fs: fs,
            });
        }

        None
    }
}

impl SessionItem for FileScheme {
    fn on_irq(&mut self, irq: u8) {
        if irq == self.fs.disk.irq {
            self.on_poll();
        }
    }

    fn on_poll(&mut self) {
        unsafe {
            self.fs.disk.on_poll();
        }
    }

    fn scheme(&self) -> String {
        return "file".to_string();
    }

    fn open(&mut self, url: &URL) -> Box<Resource> {
        let path = url.path();
        if path.len() == 0 || path.ends_with("/".to_string()) {
            let mut list = String::new();
            let mut dirs: Vec<String> = Vec::new();

            for file in self.fs.list(&path).iter() {
                let line;
                match file.find("/".to_string()) {
                    Option::Some(index) => {
                        let dirname = file.substr(0, index + 1);
                        let mut found = false;
                        for dir in dirs.iter() {
                            if dirname == *dir {
                                found = true;
                                break;
                            }
                        }
                        if found {
                            line = String::new();
                        } else {
                            line = dirname.clone();
                            dirs.push(dirname);
                        }
                    }
                    Option::None => line = file.clone(),
                }
                if line.len() > 0 {
                    if list.len() > 0 {
                        list = list + '\n' + line;
                    } else {
                        list = line;
                    }
                }
            }

            return box VecResource::new(url.clone(), ResourceType::Dir, list.to_utf8());
        } else {
            match self.fs.node(&path) {
                Option::Some(node) => {
                    let mut vec: Vec<u8> = Vec::new();
                    //TODO: Handle more extents
                    for extent in &node.extents {
                        if extent.block > 0 && extent.length > 0 {
                            unsafe {
                                let data = memory::alloc(extent.length as usize);
                                if data > 0 {
                                    let sectors = (extent.length as usize + 511) / 512;
                                    let mut sector: usize = 0;
                                    while sectors - sector >= 65536 {
                                        let request = Request {
                                            extent: Extent {
                                                block: extent.block + sector as u64,
                                                length: 65536 * 512,
                                            },
                                            mem: data + sector * 512,
                                            read: true,
                                            complete: Arc::new(AtomicBool::new(false)),
                                        };

                                        self.fs.disk.request(request.clone());

                                        while request.complete.load(Ordering::SeqCst) == false {
                                            sys_yield();
                                        }

                                        sector += 65535;
                                    }
                                    if sector < sectors {
                                        let request = Request {
                                            extent: Extent {
                                                block: extent.block + sector as u64,
                                                length: (sectors - sector) as u64 * 512,
                                            },
                                            mem: data + sector * 512,
                                            read: true,
                                            complete: Arc::new(AtomicBool::new(false)),
                                        };

                                        self.fs.disk.request(request.clone());

                                        while request.complete.load(Ordering::SeqCst) == false {
                                            sys_yield();
                                        }
                                    }

                                    vec.push_all(&Vec {
                                        data: data as *mut u8,
                                        length: extent.length as usize,
                                    });
                                }
                            }
                        }
                    }

                    return box FileResource {
                        scheme: self,
                        node: node,
                        vec: vec,
                        seek: 0,
                        dirty: false,
                    };
                }
                Option::None => {
                    return box NoneResource;
                }
            }
        }
    }
}
