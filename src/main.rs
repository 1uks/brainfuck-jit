extern crate mmap;

mod brainfuck {
    use std::{mem, ptr};
    use std::io::{Read, Write, Cursor, Seek, SeekFrom};
    use std::collections::HashMap;
    use self::Inst::*;
    use mmap::*;

    #[derive(Debug)]
    pub enum Inst {
        IncPtr,
        DecPtr,
        IncVal,
        DecVal,
        PrintCell,
        ReadChar,
        JmpFwd(usize),
        JmpBack(usize),
    }

    impl Inst {
        fn is_jmp_fwd(&self) -> bool {
            match *self {
                JmpFwd(_) => true,
                _ => false
            }
        }

        fn is_jmp_back(&self) -> bool {
            match *self {
                JmpBack(_) => true,
                _ => false
            }
        }
    }

    fn default_vec<T: Clone>(size: usize, default: T) -> Vec<T> {
        use std::iter::repeat;
        repeat(default).take(size).collect()
    }

    fn jit(insts: &[Inst]) -> Vec<u8> {
        let mut mem = Cursor::new(Vec::new());

        fn emit_inc<T: Write>(mem: &mut T) {
            mem.write(&[
                0x48, 0xff, 0xc6, // inc rsi
            ]).unwrap();
        };

        fn emit_dec<T: Write>(mem: &mut T) {
            mem.write(&[
                0x48, 0xff, 0xce, // dec rsi
            ]).unwrap();
        };

        fn emit_inc_val<T: Write>(mem: &mut T) {
            mem.write(&[
                0xfe, 0x06, // inc byte [rsi]
            ]).unwrap();
        }

        fn emit_dec_val<T: Write>(mem: &mut T) {
            mem.write(&[
                0xfe, 0x0e, // dec byte [rsi]
            ]).unwrap();
        }

        fn emit_jmp_fwd<T: Write>(mem: &mut T, offset: usize) {
            mem.write(&[
                0x80, 0x3e, 0x00, // cmp byte [rsi], 0
                0x0f, 0x84 // je ...
            ]).unwrap();
            let offset = offset as i32 - 9;
            let raw: *const u8 = unsafe { mem::transmute(&offset) };
            unsafe {
                mem.write(&[
                    *raw.offset(0),
                    *raw.offset(1),
                    *raw.offset(2),
                    *raw.offset(3),
                ]).unwrap();
            }
        }

        fn emit_jmp_back<T: Write>(mem: &mut T, offset: isize) {
            mem.write(&[
                0x80, 0x3e, 0x00, // cmp byte [rsi], 0
                0x0f, 0x85 // jne ...
            ]).unwrap();
            let offset = (offset as i32) - 9;
            let raw: *const u8 = unsafe { mem::transmute(&offset) };
            unsafe {
                mem.write(&[
                    *raw.offset(0),
                    *raw.offset(1),
                    *raw.offset(2),
                    *raw.offset(3),
                ]).unwrap();
            }
        }

        fn emit_print<T: Write>(mem: &mut T) {
            mem.write(&[
                0xb8, 0x01, 0x00, 0x00, 0x00, // mov rax, 1
                0xbf, 0x01, 0x00, 0x00, 0x00, // mov rdi, 1
                0xba, 0x01, 0x00, 0x00, 0x00, // mov edx, 1
                0x0f, 0x05 // syscall
            ]).unwrap();
        }

        fn emit_read<T: Write>(mem: &mut T) {
            mem.write(&[
                0x48, 0x31, 0xc0, // xor rax, rax
                0x48, 0x31, 0xff, // xor rdi, rdi
                0xba, 0x01, 0x00, 0x00, 0x00, // mov edx, 1
                0x0f, 0x05 // syscall
            ]).unwrap();
        }

        fn emit_ret<T: Write>(mem: &mut T) {
            mem.write(&[
                0xc3 // ret
            ]).unwrap();
        }

        let mut addr_mapping: HashMap<usize, usize> = HashMap::new();
        let mut fwd_jumps: Vec<(usize, usize)> = Vec::new();

        for (i, inst) in insts.iter().enumerate() {
            match *inst {
                IncPtr => emit_inc(&mut mem),
                DecPtr => emit_dec(&mut mem),
                IncVal => emit_inc_val(&mut mem),
                DecVal => emit_dec_val(&mut mem),
                PrintCell => emit_print(&mut mem),
                ReadChar => emit_read(&mut mem),
                JmpFwd(n) => {
                    fwd_jumps.push((mem.position() as usize, n));
                    emit_jmp_fwd(&mut mem, 0x41414141); // insert dummy
                    addr_mapping.insert(i, mem.position() as usize);
                },
                JmpBack(n) => {
                    let distance = mem.position() as isize - addr_mapping[&n] as isize;
                    emit_jmp_back(&mut mem, -distance);
                    addr_mapping.insert(i, mem.position() as usize);
                },
            }
        }

        for (offset, n) in fwd_jumps {
            mem.set_position(offset as u64);
            let distance = addr_mapping[&n] - offset;
            emit_jmp_fwd(&mut mem, distance);
        }

        mem.seek(SeekFrom::End(0)).unwrap();
        emit_ret(&mut mem);

        mem.into_inner()
    }

    pub struct Brainfuck {
        insts: Vec<Inst>,
        jit_code: Vec<u8>,
        tape_size: usize,
    }

    #[derive(Debug)]
    pub enum CompileError {
        UnbalancedBrackets,
        InvalidInst,
    }

    impl Brainfuck {
        pub fn new(program: &str) -> Result<Brainfuck, CompileError> {
            use self::CompileError::*;

            let mut insts = Vec::new();
            let mut stack = Vec::new();

            let program: String = program.chars().filter(
                |&c| match c {
                    '>' | '<' | '+' | '-' | '.' | ',' | '[' | ']' => true,
                    _ => false,
                }
            ).collect();

            for (i, c) in program.chars().enumerate() {
                let inst = match c {
                    '>' => IncPtr,
                    '<' => DecPtr,
                    '+' => IncVal,
                    '-' => DecVal,
                    '.' => PrintCell,
                    ',' => ReadChar,
                    '[' => {
                        stack.push(i);
                        JmpFwd(0) // insert dummy
                    },
                    ']' => {
                        let n = match stack.pop() {
                            Some(n) => n,
                            None => return Err(UnbalancedBrackets),
                        };
                        insts[n] = JmpFwd(i);
                        JmpBack(n)
                    },
                    _ => return Err(InvalidInst)
                };

                insts.push(inst);
            }

            if !stack.is_empty() {
                return Err(UnbalancedBrackets);
            }

            Ok(Brainfuck {
                jit_code: jit(&insts),
                insts: insts,
                tape_size: 30_000,
            })
        }

        pub fn tape_size(&self) -> usize {
            self.tape_size
        }

        pub fn set_tape_size(&mut self, size: usize) {
            self.tape_size = size;
        }

        pub fn run(&mut self) {
            let tape = default_vec(self.tape_size, 0u8);
            let rwx = &[
                MapOption::MapReadable,
                MapOption::MapWritable,
                MapOption::MapExecutable
            ];
            let mapping = MemoryMap::new(self.jit_code.len(), rwx).unwrap();
            unsafe {
                ptr::copy(self.jit_code.as_ptr(), mapping.data(), self.jit_code.len());
            }
            let func: fn(*const u8, *const u8) = unsafe {
                mem::transmute(mapping.data())
            };
            func(ptr::null(), tape.as_ptr());  // jitted code expects tape in rsi
        }

        pub fn dump(&self) {
            let mut shift = 0;
            let indent = "    ";
            for (i, inst) in self.insts.iter().enumerate() {
                if inst.is_jmp_back() {
                    shift -= 1;
                }
                for _ in 0..shift {
                    print!("{}", indent);
                }
                println!("{}: {:?}", i, inst);
                if inst.is_jmp_fwd() {
                    shift += 1;
                }
            }
        }

        pub fn dump_jit(&self) {
            io::stdout().write(&self.jit_code);
        }

    }
}


fn main() {
    use std::{env, fs, path};
    use std::path::Path;
    use std::fs::File;
    use std::io::Read;
    use brainfuck::*;

    let args: Vec<_> = env::args().collect();

    if args.len() != 2 {
        let progname = Path::new(&args[0]).file_name().unwrap().to_string_lossy();
        println!("usage: {} <filename>", progname);
        return;
    }

    let mut code = String::new();

    File::open(&args[1]).unwrap().read_to_string(&mut code);

    Brainfuck::new(&code).unwrap().run();
}
