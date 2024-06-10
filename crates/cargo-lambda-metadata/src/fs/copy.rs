// Code adapted from https://github.com/rust-lang/rustup

// LICENSE:
// Copyright (c) 2016 The Rust Project Developers
//
// Permission is hereby granted, free of charge, to any
// person obtaining a copy of this software and associated
// documentation files (the "Software"), to deal in the
// Software without restriction, including without
// limitation the rights to use, copy, modify, merge,
// publish, distribute, sublicense, and/or sell copies of
// the Software, and to permit persons to whom the Software
// is furnished to do so, subject to the following
// conditions:
//
// The above copyright notice and this permission notice
// shall be included in all copies or substantial portions
// of the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF
// ANY KIND, EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED
// TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A
// PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT
// SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY
// CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION
// OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR
// IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER
// DEALINGS IN THE SOFTWARE.
//

use std::{fs, io, path::Path};

pub fn copy_and_replace<P, Q>(from: P, to: Q) -> io::Result<()>
where
    P: AsRef<Path>,
    Q: AsRef<Path>,
{
    copy_and_delete(from, to, true)
}

pub fn copy_without_replace<P, Q>(from: P, to: Q) -> io::Result<()>
where
    P: AsRef<Path>,
    Q: AsRef<Path>,
{
    copy_and_delete(from, to, false)
}

fn copy_and_delete<P, Q>(from: P, to: Q, replace: bool) -> io::Result<()>
where
    P: AsRef<Path>,
    Q: AsRef<Path>,
{
    let from = from.as_ref();
    if from.is_dir() {
        return copy_dir(from, to, replace).and(remove_dir_all::remove_dir_all(from));
    } else if replace || !to.as_ref().exists() {
        fs::copy(from, to)?;
    }

    if replace {
        fs::remove_file(from)?;
    }

    Ok(())
}

fn copy_dir<P, Q>(from: P, to: Q, replace: bool) -> io::Result<()>
where
    P: AsRef<Path>,
    Q: AsRef<Path>,
{
    fs::create_dir_all(&to)?;
    for entry in from.as_ref().read_dir()? {
        let entry = entry?;
        let kind = entry.file_type()?;
        let from = entry.path();

        let to = to.as_ref().join(entry.file_name());
        if kind.is_dir() {
            copy_dir(&from, &to, replace)?;
        } else if replace || !to.exists() {
            fs::copy(&from, &to)?;
        }
    }
    Ok(())
}
