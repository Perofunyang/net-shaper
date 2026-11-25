use std::env;
use std::fs;
use std::path::{Path, PathBuf};

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let windivert_dir = manifest_dir.join("vendor").join("WinDivert").join("x64");
    println!("cargo:rerun-if-changed={}", windivert_dir.display());

    let lib_file = windivert_dir.join("WinDivert.lib");
    let dll_file = windivert_dir.join("WinDivert.dll");
    let sys_file = windivert_dir.join("WinDivert64.sys");

    if !lib_file.exists() || !dll_file.exists() || !sys_file.exists() {
        panic!(
            "\n[WinDivert 파일 누락]\n{}\n경로에 WinDivert.lib, WinDivert.dll, WinDivert64.sys 3개 파일을 넣어주세요.\n",
            windivert_dir.display()
        );
    }

    println!("cargo:rustc-link-search=native={}", windivert_dir.display());
    println!("cargo:rustc-link-lib=WinDivert");

    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());

    // target/debug 또는 target/release 폴더로 DLL / SYS 자동 복사
    if let Some(target_profile_dir) = Path::new(&out_dir).ancestors().nth(3) {
        for file in [&dll_file, &sys_file] {
            if let Some(name) = file.file_name() {
                let dest = target_profile_dir.join(name);
                let need_copy = match (fs::metadata(file), fs::metadata(&dest)) {
                    (Ok(src_m), Ok(dst_m)) => src_m.len() != dst_m.len(),
                    _ => true,
                };
                if need_copy {
                    let _ = fs::copy(file, &dest);
                }
            }
        }
    }

    // 1) 창 타이틀바용 64x64 RGBA + Windows EXE용 멀티사이즈 ICO 자동 생성
    let rgba_64 = render_icon_rgba(64);
    fs::write(out_dir.join("icon_64.rgba"), &rgba_64).unwrap();

    let ico_images: Vec<(u32, Vec<u8>)> = [16, 32, 48, 256]
        .into_iter()
        .map(|s| (s, render_icon_rgba(s)))
        .collect();
    let ico_bytes = encode_ico(&ico_images);
    let ico_path = out_dir.join("netshaper.ico");
    fs::write(&ico_path, ico_bytes).unwrap();

    // 2) Windows 리소스(아이콘 + UAC 관리자 매니페스트 + 버전 정보) 컴파일
    if env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        let ico_str = ico_path.to_string_lossy().replace('\\', "/");
        res.set_icon(&ico_str);
        res.set_manifest(
            r#"<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <trustInfo xmlns="urn:schemas-microsoft-com:asm.v3">
    <security>
      <requestedPrivileges>
        <requestedExecutionLevel level="requireAdministrator" uiAccess="false" />
      </requestedPrivileges>
    </security>
  </trustInfo>
  <compatibility xmlns="urn:schemas-microsoft-com:compatibility.v1">
    <application>
      <supportedOS Id="{8e0f7a12-bfb3-4fe8-b9a5-48fd50a15a9a}"/>
    </application>
  </compatibility>
</assembly>"#,
        );
        res.compile().expect("Windows 리소스 컴파일 실패");
    }
}

fn sd_rounded_box(px: f32, py: f32, hx: f32, hy: f32, r: f32) -> f32 {
    let qx = px.abs() - hx + r;
    let qy = py.abs() - hy + r;
    qx.max(0.0).hypot(qy.max(0.0)) + qx.max(qy).min(0.0) - r
}

fn blend(bg: [f32; 4], fg: [f32; 4]) -> [f32; 4] {
    let sa = fg[3];
    let da = bg[3];
    let out_a = sa + da * (1.0 - sa);
    if out_a <= 1e-6 {
        return [0.0; 4];
    }
    [
        (fg[0] * sa + bg[0] * da * (1.0 - sa)) / out_a,
        (fg[1] * sa + bg[1] * da * (1.0 - sa)) / out_a,
        (fg[2] * sa + bg[2] * da * (1.0 - sa)) / out_a,
        out_a,
    ]
}

fn render_icon_rgba(size: u32) -> Vec<u8> {
    let mut out = vec![0u8; (size * size * 4) as usize];
    let r_in = 0.255f32;
    let sub = 4u32;

    for y in 0..size {
        for x in 0..size {
            let mut acc = [0.0f32; 4];
            for sy in 0..sub {
                for sx in 0..sub {
                    let u = (x as f32 + (sx as f32 + 0.5) / sub as f32) / size as f32;
                    let v = (y as f32 + (sy as f32 + 0.5) / sub as f32) / size as f32;

                    let dx = u - 0.5;
                    let dy = v - 0.5;
                    let sd_bg = sd_rounded_box(dx, dy, 0.44, 0.44, 0.20);
                    let mut col = [0.0f32; 4];

                    if sd_bg <= 0.0 {
                        // 매트한 다크 슬레이트 배경 (#1E2430)
                        col = [0.118, 0.141, 0.188, 1.0];

                        // 얇고 은은한 테두리 (#384254)
                        if sd_bg > -0.028 {
                            col = blend(col, [0.220, 0.259, 0.329, 1.0]);
                        }

                        // 중앙 미니멀 3면 큐브 (무채색 화이트/그레이)
                        let adx = dx.abs();
                        let ady = dy.abs();
                        if adx <= r_in && (adx * 0.5 + ady * 0.8660254) <= r_in {
                            col = if dy < -(adx * 0.57735027) {
                                [0.973, 0.980, 0.988, 1.0] // 윗면: 화이트 (#F8FAFC)
                            } else if dx < 0.0 {
                                [0.796, 0.835, 0.882, 1.0] // 왼쪽: 라이트 그레이 (#CBD5E1)
                            } else {
                                [0.486, 0.545, 0.631, 1.0] // 오른쪽: 슬레이트 (#7C8BA1)
                            };
                        }
                    }

                    acc[0] += col[0] * col[3];
                    acc[1] += col[1] * col[3];
                    acc[2] += col[2] * col[3];
                    acc[3] += col[3];
                }
            }

            let samples = (sub * sub) as f32;
            let a = (acc[3] / samples).clamp(0.0, 1.0);
            let idx = ((y * size + x) * 4) as usize;
            if a > 1e-6 {
                out[idx] = ((acc[0] / acc[3]).clamp(0.0, 1.0) * 255.0).round() as u8;
                out[idx + 1] = ((acc[1] / acc[3]).clamp(0.0, 1.0) * 255.0).round() as u8;
                out[idx + 2] = ((acc[2] / acc[3]).clamp(0.0, 1.0) * 255.0).round() as u8;
                out[idx + 3] = (a * 255.0).round() as u8;
            }
        }
    }
    out
}

fn encode_ico(images: &[(u32, Vec<u8>)]) -> Vec<u8> {
    let count = images.len() as u16;
    let mut out = Vec::new();
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&count.to_le_bytes());

    let mut offset: u32 = 6 + (images.len() as u32) * 16;
    let mut payloads = Vec::with_capacity(images.len());

    for (size, rgba) in images {
        let w = *size;
        let h = *size;
        let mask_row_bytes = ((w + 31) / 32) * 4;
        let xor_bytes = w * h * 4;
        let and_bytes = mask_row_bytes * h;
        let image_bytes = 40 + xor_bytes + and_bytes;

        out.push(if w >= 256 { 0 } else { w as u8 });
        out.push(if h >= 256 { 0 } else { h as u8 });
        out.push(0);
        out.push(0);
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&32u16.to_le_bytes());
        out.extend_from_slice(&image_bytes.to_le_bytes());
        out.extend_from_slice(&offset.to_le_bytes());
        offset += image_bytes;

        let mut img = Vec::with_capacity(image_bytes as usize);
        img.extend_from_slice(&40u32.to_le_bytes());
        img.extend_from_slice(&(w as i32).to_le_bytes());
        img.extend_from_slice(&((h * 2) as i32).to_le_bytes());
        img.extend_from_slice(&1u16.to_le_bytes());
        img.extend_from_slice(&32u16.to_le_bytes());
        img.extend_from_slice(&0u32.to_le_bytes());
        img.extend_from_slice(&(xor_bytes as u32).to_le_bytes());
        img.extend_from_slice(&0i32.to_le_bytes());
        img.extend_from_slice(&0i32.to_le_bytes());
        img.extend_from_slice(&0u32.to_le_bytes());
        img.extend_from_slice(&0u32.to_le_bytes());

        for y in (0..h).rev() {
            for x in 0..w {
                let idx = ((y * w + x) * 4) as usize;
                img.push(rgba[idx + 2]);
                img.push(rgba[idx + 1]);
                img.push(rgba[idx]);
                img.push(rgba[idx + 3]);
            }
        }

        for y in (0..h).rev() {
            let mut row = vec![0u8; mask_row_bytes as usize];
            for x in 0..w {
                let idx = ((y * w + x) * 4) as usize;
                if rgba[idx + 3] == 0 {
                    row[(x / 8) as usize] |= 1 << (7 - (x % 8));
                }
            }
            img.extend_from_slice(&row);
        }
        payloads.push(img);
    }

    for p in payloads {
        out.extend_from_slice(&p);
    }
    out
}