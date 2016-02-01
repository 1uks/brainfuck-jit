extern crate mmap;
extern crate clap;

mod runlength;

#[allow(dead_code)]
#[allow(unused_must_use)]
mod brainfuck {
    use std::{mem, ptr, io};
    use std::io::{Read, Write, Cursor, Seek, SeekFrom};
    use std::collections::HashMap;
    use self::Inst::*;
    use mmap::*;
    use runlength::RunLengthIterator;

    #[derive(Debug)]
    pub enum Inst {
        IncPtr(usize),
        DecPtr(usize),
        IncVal(usize),
        DecVal(usize),
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

    fn compile(insts: &[Inst]) -> Vec<u8> {
        let mut mem = Cursor::new(Vec::new());

        fn emit_inc<T: Write>(mem: &mut T, amount: usize) {
            if amount == 1 {
                mem.write(&[
                    0x48, 0xff, 0xc6, // inc rsi
                ]);
            } else {
                let raw: *const u8 = unsafe { mem::transmute(&(amount as u32)) };
                unsafe {
                    mem.write(&[
                        0x48, 0x81, 0xc6,
                        *raw.offset(0),
                        *raw.offset(1),
                        *raw.offset(2),
                        *raw.offset(3),
                    ]);
                }
            }
        };

        fn emit_dec<T: Write>(mem: &mut T, amount: usize) {
            if amount == 1 {
                mem.write(&[
                    0x48, 0xff, 0xce, // dec rsi
                ]);
            } else {
                let raw: *const u8 = unsafe { mem::transmute(&(amount as u32)) };
                unsafe {
                    mem.write(&[
                        0x48, 0x81, 0xee,
                        *raw.offset(0),
                        *raw.offset(1),
                        *raw.offset(2),
                        *raw.offset(3),
                    ]);
                }
            }
        };

        fn emit_inc_val<T: Write>(mem: &mut T, amount: usize) {
            if amount == 1 {
                mem.write(&[
                    0xfe, 0x06, // inc byte [rsi]
                ]);
            } else {
                let raw: *const u8 = unsafe { mem::transmute(&((amount & 0xff) as u8)) };
                unsafe {
                    mem.write(&[
                        0x80, 0x06, *raw.offset(0)
                    ]);
                }
            }
        }

        fn emit_dec_val<T: Write>(mem: &mut T, amount: usize) {
            if amount == 1 {
                mem.write(&[
                    0xfe, 0x0e, // dec byte [rsi]
                ]);
            } else {
                let raw: *const u8 = unsafe { mem::transmute(&((amount & 0xff) as u8)) };
                unsafe {
                    mem.write(&[
                        0x80, 0x2e, *raw.offset(0)
                    ]);
                }
            }
        }

        fn emit_jmp_fwd<T: Write>(mem: &mut T, offset: usize) {
            mem.write(&[
                0x80, 0x3e, 0x00, // cmp byte [rsi], 0
                0x0f, 0x84 // je ...
            ]);
            let offset = offset as i32 - 9;
            let raw: *const u8 = unsafe { mem::transmute(&offset) };
            unsafe {
                mem.write(&[
                    *raw.offset(0),
                    *raw.offset(1),
                    *raw.offset(2),
                    *raw.offset(3),
                ]);
            }
        }

        fn emit_jmp_back<T: Write>(mem: &mut T, offset: isize) {
            mem.write(&[
                0x80, 0x3e, 0x00, // cmp byte [rsi], 0
                0x0f, 0x85 // jne ...
            ]);
            let offset = (offset as i32) - 9;
            let raw: *const u8 = unsafe { mem::transmute(&offset) };
            unsafe {
                mem.write(&[
                    *raw.offset(0),
                    *raw.offset(1),
                    *raw.offset(2),
                    *raw.offset(3),
                ]);
            }
        }

        fn emit_print<T: Write>(mem: &mut T) {
            mem.write(&[
                0xb8, 0x01, 0x00, 0x00, 0x00, // mov rax, 1
                0xbf, 0x01, 0x00, 0x00, 0x00, // mov rdi, 1
                0xba, 0x01, 0x00, 0x00, 0x00, // mov edx, 1
                0x0f, 0x05 // syscall
            ]);
        }

        fn emit_read<T: Write>(mem: &mut T) {
            mem.write(&[
                0x48, 0x31, 0xc0, // xor rax, rax
                0x48, 0x31, 0xff, // xor rdi, rdi
                0xba, 0x01, 0x00, 0x00, 0x00, // mov edx, 1
                0x0f, 0x05 // syscall
            ]);
        }

        fn emit_ret<T: Write>(mem: &mut T) {
            mem.write(&[
                0xc3 // ret
            ]);
        }

        let mut addr_mapping: HashMap<usize, usize> = HashMap::new();
        let mut fwd_jumps: Vec<(usize, usize)> = Vec::new();

        for (i, inst) in insts.iter().enumerate() {
            match *inst {
                IncPtr(a) => emit_inc(&mut mem, a),
                DecPtr(a) => emit_dec(&mut mem, a),
                IncVal(a) => emit_inc_val(&mut mem, a),
                DecVal(a) => emit_dec_val(&mut mem, a),
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

            for (length, c) in program.chars().run_length() {

                match c {
                    '>' => insts.push(IncPtr(length)),
                    '<' => insts.push(DecPtr(length)),
                    '+' => insts.push(IncVal(length)),
                    '-' => insts.push(DecVal(length)),
                    '.' => {
                        for _ in 0..length {
                            insts.push(PrintCell);
                        }
                    }
                    ',' => {
                        for _ in 0..length {
                            insts.push(ReadChar);
                        }
                    }
                    '[' => {
                        for _ in 0..length {
                            stack.push(insts.len());
                            insts.push(JmpFwd(0)); // insert dummy;
                        }
                    },
                    ']' => {
                        for _ in 0..length {
                            let n = match stack.pop() {
                                Some(n) => n,
                                None => return Err(UnbalancedBrackets),
                            };
                            insts[n] = JmpFwd(insts.len());
                            insts.push(JmpBack(n));
                        }
                    },
                    _ => unreachable!(),
                };

            }

            if !stack.is_empty() {
                return Err(UnbalancedBrackets);
            }

            Ok(Brainfuck {
                jit_code: compile(&insts),
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


#[cfg(target_arch="x86_64")]
fn main() {
    use std::fs::File;
    use std::io::Read;
    use brainfuck::*;
    use clap::{App, Arg};

    let matches = App::new("brainfuck-jit")
        .arg(Arg::with_name("filename").required(true))
        .get_matches();

    let mut code = String::new();

    File::open(matches.value_of("filename").unwrap()).unwrap()
        .read_to_string(&mut code).unwrap();

    Brainfuck::new(&code).unwrap().run();
}
