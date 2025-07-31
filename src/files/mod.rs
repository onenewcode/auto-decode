use anyhow::{Context, Result};
use std::collections::HashMap;
use std::fs;
use std::{fs::File, path::Path};



// 假设 rename_file 函数签名如下（根据上下文推断）：
// fn rename_file(path: &Path, rename_hash: &HashMap<String, String>) -> Result<File> { ... }

pub fn get_file_handles<P: AsRef<Path>>(
    path: P,
    rename_hash: &HashMap<String, String>,
) -> Result<Vec<File>> {
    let path = path.as_ref();
    let metadata = fs::metadata(path)
        .with_context(|| format!("无法获取路径 {} 的元数据", path.display()))?;

    if metadata.is_file() {
        // 单个文件：直接处理并包装成 Vec
        rename_file(path, rename_hash).map(|file| vec![file])
    } else if metadata.is_dir() {
        // 处理目录：收集所有文件结果
        let entries = fs::read_dir(path)
            .with_context(|| format!("无法读取目录 {}", path.display()))?;

        let mut files = Vec::new();
        for entry in entries {
            let entry = entry?;
            let path = entry.path();
            if path.is_file() {
                // 处理文件并收集结果
                files.push(rename_file(&path, rename_hash)?);
            }
        }
        Ok(files)
    } else {
        anyhow::bail!("路径 {} 不是文件也不是目录", path.display());
    }
}
#[inline]
pub fn rename_file<P: AsRef<Path>>(path: P, rename_hash: &HashMap<String, String>) -> Result<File> {
    let original_path = path.as_ref();

    // 获取扩展名（无扩展名时直接打开原文件）
    let Some(extension) = original_path.extension().and_then(|ext| ext.to_str()) else {
        anyhow::bail!("文件 {} 没有扩展名", original_path.display())
    };

    // 直接获取新文件名（避免 contains_key + get 的双重查找）
    match rename_hash.get(extension) {
        Some(name) => {
            let new_path = original_path.with_extension(name);
            // 执行重命名操作
            fs::rename(original_path, &new_path).with_context(|| {
                format!(
                    "Failed to rename {} to {}",
                    original_path.display(),
                    new_path.display()
                )
            })?;
            File::open(&new_path)
                .with_context(|| format!("无法打开重命名后的文件 {}", new_path.display()))
        }
        None => {
            // 如果没有对应的重命名规则，直接打开原文件
            File::open(original_path)
                .with_context(|| format!("无法打开原文件 {}", original_path.display()))
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::fs::{self, File};
    use std::io::Write;
    use tempfile::tempdir;

    #[test]
    fn test_rename_with_extension_in_map() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let file_path = dir.path().join("test.txt");
        let mut file = File::create(&file_path)?;
        writeln!(file, "content")?;

        let mut rename_map = HashMap::new();
        rename_map.insert("txt".to_string(), "md".to_string());

        let result = rename_file(&file_path, &rename_map)?;
        assert!(result.metadata().is_ok());

        // 验证原文件已不存在
        assert!(!file_path.exists());

        // 验证新文件存在且内容正确
        let new_path = dir.path().join("test.md");
        assert!(new_path.exists());
        let content = fs::read_to_string(&new_path)?;
        assert_eq!(content.trim(), "content");

        Ok(())
    }
}
