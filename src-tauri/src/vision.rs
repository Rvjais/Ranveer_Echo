//! Screen reading via the built-in Windows OCR engine (Windows.Media.Ocr).
//!
//! This is the local, low-footprint "what's on my screen" path chosen for
//! modest hardware: capture the screen to a PNG, OCR it with the OS engine (no
//! model download, near-instant), then let the local text model answer the
//! user's question about the extracted text. A heavier multimodal Ollama model
//! could describe scenes, but OCR covers "read my screen" well without one.

/// PowerShell that screenshots the primary screen and OCRs it. `__IMG__` is
/// replaced with the temp PNG path. Uses the WinRT async `Await` pattern.
#[cfg(windows)]
const OCR_SCRIPT: &str = r#"
$ErrorActionPreference='Stop'
Add-Type -AssemblyName System.Windows.Forms,System.Drawing
$b=[System.Windows.Forms.Screen]::PrimaryScreen.Bounds
$bmp=New-Object System.Drawing.Bitmap $b.Width,$b.Height
$g=[System.Drawing.Graphics]::FromImage($bmp)
$g.CopyFromScreen($b.Location,[System.Drawing.Point]::Empty,$b.Size)
$bmp.Save('__IMG__')
$g.Dispose(); $bmp.Dispose()
Add-Type -AssemblyName System.Runtime.WindowsRuntime
$asTask=[System.WindowsRuntimeSystemExtensions].GetMethods() | Where-Object { $_.Name -eq 'AsTask' -and $_.GetParameters().Count -eq 1 -and $_.GetParameters()[0].ParameterType.Name -eq 'IAsyncOperation`1' } | Select-Object -First 1
function Await($op,$t){ $m=$asTask.MakeGenericMethod($t); $task=$m.Invoke($null,@($op)); $task.Wait(-1)|Out-Null; $task.Result }
[Windows.Media.Ocr.OcrEngine,Windows.Media,ContentType=WindowsRuntime]|Out-Null
[Windows.Storage.StorageFile,Windows.Storage,ContentType=WindowsRuntime]|Out-Null
[Windows.Graphics.Imaging.BitmapDecoder,Windows.Graphics.Imaging,ContentType=WindowsRuntime]|Out-Null
$file=Await ([Windows.Storage.StorageFile]::GetFileFromPathAsync('__IMG__')) ([Windows.Storage.StorageFile])
$stream=Await ($file.OpenAsync([Windows.Storage.FileAccessMode]::Read)) ([Windows.Storage.Streams.IRandomAccessStream])
$decoder=Await ([Windows.Graphics.Imaging.BitmapDecoder]::CreateAsync($stream)) ([Windows.Graphics.Imaging.BitmapDecoder])
$bitmap=Await ($decoder.GetSoftwareBitmapAsync()) ([Windows.Graphics.Imaging.SoftwareBitmap])
$engine=[Windows.Media.Ocr.OcrEngine]::TryCreateFromUserProfileLanguages()
if($engine -eq $null){ Write-Error 'No OCR language is installed on this system.'; exit 1 }
$result=Await ($engine.RecognizeAsync($bitmap)) ([Windows.Media.Ocr.OcrResult])
[Console]::Out.WriteLine($result.Text)
"#;

/// Captures the primary screen and returns its OCR text (may be empty).
/// Blocking — call via `spawn_blocking` from async contexts.
pub fn capture_and_ocr() -> Result<String, String> {
    #[cfg(windows)]
    {
        let img = std::env::temp_dir().join(format!("ranveer_screen_{}.png", std::process::id()));
        let img_str = img.to_string_lossy().replace('\'', "''");
        let script = OCR_SCRIPT.replace("__IMG__", &img_str);
        let script_path =
            std::env::temp_dir().join(format!("ranveer_ocr_{}.ps1", std::process::id()));
        std::fs::write(&script_path, script).map_err(|e| e.to_string())?;

        let out = std::process::Command::new("powershell")
            .args([
                "-NoProfile",
                "-ExecutionPolicy",
                "Bypass",
                "-File",
                &script_path.to_string_lossy(),
            ])
            .output()
            .map_err(|e| e.to_string())?;

        let _ = std::fs::remove_file(&script_path);
        let _ = std::fs::remove_file(&img);

        let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if text.is_empty() {
            let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
            if !err.is_empty() {
                return Err(err);
            }
        }
        Ok(text)
    }
    #[cfg(not(windows))]
    {
        Err("Screen OCR is only supported on Windows.".to_string())
    }
}

/// Answers a natural-language question about what's on screen using OCR text +
/// the local text model.
pub async fn answer_about_screen(question: &str) -> String {
    let ocr = match tokio::task::spawn_blocking(capture_and_ocr).await {
        Ok(Ok(t)) => t,
        Ok(Err(e)) => return format!("I couldn't read the screen: {e}"),
        Err(_) => return "I couldn't read the screen (capture task failed).".to_string(),
    };
    if ocr.is_empty() {
        return "I captured the screen but couldn't read any text on it, sir.".to_string();
    }
    // Keep the prompt within the small model's context.
    let clipped: String = ocr.chars().take(4000).collect();
    let system = "You are Ranveer. You are given the OCR text extracted from the user's screen. Answer their question about it concisely — at most two sentences. If the answer is not present in the text, say you couldn't find it on the screen.";
    let prompt = format!("Screen text:\n{clipped}\n\nQuestion: {question}\n\nAnswer:");
    match crate::ai::chat(&prompt, Some(system), 300, 0.4).await {
        Ok(r) => r.trim().to_string(),
        Err(e) => format!("My model is unreachable, so I can't analyze the screen right now: {e}"),
    }
}
