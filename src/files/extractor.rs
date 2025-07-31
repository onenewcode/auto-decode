use std::{
    fs::{File, OpenOptions},
    io::{self, BufReader, BufWriter, Read, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Instant,
};
use zip::{read::ZipArchive, result::ZipError};

/// 高性能 ZIP 解压器（专为大文件优化）
pub struct ZipExtractor {
    /// 输入 ZIP 文件路径
    zip_path: PathBuf,
    /// 输出目录
    output_dir: PathBuf,
    /// 读缓冲区大小 (字节)
    read_buffer_size: usize,
    /// 写缓冲区大小 (字节)
    write_buffer_size: usize,
    /// 并行解压线程数 (0=自动选择)
    worker_threads: usize,
}

impl ZipExtractor {
    /// 创建新的解压器实例
    pub fn new<P: AsRef<Path>>(zip_path: P, output_dir: P) -> Self {
        Self {
            zip_path: zip_path.as_ref().to_path_buf(),
            output_dir: output_dir.as_ref().to_path_buf(),
            read_buffer_size: 2 * 1024 * 1024,  // 默认 2MB 读缓冲
            write_buffer_size: 4 * 1024 * 1024, // 默认 4MB 写缓冲
            worker_threads: 0,                  // 自动选择线程数
        }
    }

    /// 设置读缓冲区大小 (字节)
    pub fn read_buffer_size(mut self, size: usize) -> Self {
        self.read_buffer_size = size;
        self
    }

    /// 设置写缓冲区大小 (字节)
    pub fn write_buffer_size(mut self, size: usize) -> Self {
        self.write_buffer_size = size;
        self
    }

    /// 设置工作线程数
    pub fn worker_threads(mut self, count: usize) -> Self {
        self.worker_threads = count;
        self
    }

    /// 执行解压操作（返回解压耗时）
    pub fn extract(&self) -> Result<f64, ZipError> {
        let start_time = Instant::now();

        // 打开 ZIP 文件并使用大缓冲区
        let file = File::open(&self.zip_path)?;
        let reader = BufReader::with_capacity(self.read_buffer_size, file);
        let mut archive = ZipArchive::new(reader)?;

        // 确定最佳线程数
        let num_files = archive.len();
        let num_threads = match self.worker_threads {
            0 => (num_files / 20).clamp(1, num_cpus::get()), // 每20个文件一个线程
            n => n.min(num_files),
        };

        if num_threads > 1 {
            self.extract_parallel(&mut archive, num_threads)?;
        } else {
            self.extract_sequential(&mut archive)?;
        }

        let duration = start_time.elapsed().as_secs_f64();
        Ok(duration)
    }

    /// 顺序解压（单线程）
    fn extract_sequential(
        &self,
        archive: &mut ZipArchive<BufReader<File>>,
    ) -> Result<(), ZipError> {
        for i in 0..archive.len() {
            let mut file = archive.by_index(i)?;
            let out_path = self.output_dir.join(file.sanitized_name());

            if file.is_dir() {
                std::fs::create_dir_all(&out_path)?;
            } else {
                self.extract_file(&mut file, &out_path)?;
            }
        }
        Ok(())
    }

    /// 并行解压（多线程）
    fn extract_parallel(
        &self,
        archive: &mut ZipArchive<BufReader<File>>,
        num_threads: usize,
    ) -> Result<(), ZipError> {
        // 创建线程池
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(num_threads)
            .build()
            .unwrap();

        // 创建文件索引列表
        let file_indices: Vec<usize> = (0..archive.len()).collect();

        // 共享 ZIP 文件的原子引用计数器
        let archive_mutex = Arc::new(Mutex::new(archive));

        pool.scope(|s| {
            for chunk in file_indices.chunks(file_indices.len() / num_threads + 1) {
                let archive_ref = Arc::clone(&archive_mutex);
                let extractor = self; // 借用 self

                s.spawn(move |_| {
                    for &index in chunk {
                        let mut archive = archive_ref.lock().unwrap();
                        let mut file = match archive.by_index(index) {
                            Ok(f) => f,
                            Err(e) => {
                                eprintln!("Error accessing file {}: {}", index, e);
                                continue;
                            }
                        };

                        let out_path = extractor.output_dir.join(file.sanitized_name());

                        if file.is_dir() {
                            if let Err(e) = std::fs::create_dir_all(&out_path) {
                                eprintln!("Error creating directory {:?}: {}", out_path, e);
                            }
                        } else {
                            if let Err(e) = extractor.extract_file(&mut file, &out_path) {
                                eprintln!("Error extracting file {:?}: {}", out_path, e);
                            }
                        }
                    }
                });
            }
        });

        Ok(())
    }

    /// 提取单个文件（核心提取逻辑）
    fn extract_file<R: Read>(&self, reader: &mut R, output_path: &Path) -> Result<(), io::Error> {
        // 确保父目录存在
        if let Some(parent) = output_path.parent() {
            if !parent.exists() {
                std::fs::create_dir_all(parent)?;
            }
        }

        // 使用缓冲写入器
        let file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(output_path)?;

        let mut writer = BufWriter::with_capacity(self.write_buffer_size, file);

        // 使用大缓冲区拷贝数据
        let mut buffer = vec![0u8; 64 * 1024]; // 64KB 拷贝缓冲区
        while let Ok(n) = reader.read(&mut buffer) {
            if n == 0 {
                break;
            }
            writer.write_all(&buffer[..n])?;
        }

        writer.flush()?;
        Ok(())
    }
}

// 为 zip::read::ZipFile 添加 sanitized_name 方法
trait SafeName {
    fn sanitized_name(&self) -> PathBuf;
}

impl<'a> SafeName for zip::read::ZipFile<'a> {
    fn sanitized_name(&self) -> PathBuf {
        self.name()
            .split('/')
            .filter(|s| !s.is_empty() && *s != "..")
            .fold(PathBuf::new(), |mut path, comp| {
                path.push(comp);
                path
            })
    }
}
