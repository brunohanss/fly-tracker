$ErrorActionPreference = 'Stop'
$taskRoot = (Resolve-Path (Join-Path $PSScriptRoot '../..')).Path
$taskPython = Join-Path $taskRoot '.venv-safety/Scripts/python.exe'
$taskModels = Join-Path $taskRoot 'models/nanodet-reference'
$expectedHashes = @{
    'coco.torchscript.ncnn.param' = '1C0DDF1E8009A6FF55032DEDA21B97E221EBDFE68D820FAA68999790D5E972F9'
    'coco.torchscript.ncnn.bin' = '1942DEDE585C2163C888E34295FFFFFDF73841CA8E79B2A548042393510FB9AE'
}
foreach ($modelName in $expectedHashes.Keys) {
    $actualHash = (Get-FileHash -LiteralPath (Join-Path $taskModels $modelName) -Algorithm SHA256).Hash
    if ($actualHash -ne $expectedHashes[$modelName]) { throw "Model checksum mismatch: $modelName" }
}
$expectedImages = @{
    'bus.jpg' = 'C02019C4979C191EB739DDD944445EF408DAD5679ACAB6FD520EF9D434BFBC63'
    'dog.jpg' = 'F3F87BB8AB3C26C7ECFD3AC60421D7F32B0503D1D6C5BAF8BAC42ED93D86351A'
}
foreach ($imageName in $expectedImages.Keys) {
    $imagePath = Join-Path $taskRoot ".test-dist/nanodet/$imageName"
    if ((Get-FileHash -LiteralPath $imagePath -Algorithm SHA256).Hash -ne $expectedImages[$imageName]) {
        throw "Smoke image checksum mismatch: $imageName"
    }
}
$oldTestPython = $env:FLY_TRACKER_TEST_PYTHON
$oldTestImages = $env:FLY_TRACKER_NANODET_IMAGES
Push-Location $taskRoot
try {
    & $taskPython tools/safety/prepare_test_images.py .test-dist/nanodet
    if ($LASTEXITCODE -ne 0) { throw 'Image preparation failed' }
    $env:FLY_TRACKER_TEST_PYTHON = $taskPython
    $env:FLY_TRACKER_NANODET_IMAGES = Join-Path $taskRoot '.test-dist/nanodet/real-images.json'
    cargo test -p safety --test nanodet_inference --test nanodet_process -- --ignored
    if ($LASTEXITCODE -ne 0) { throw 'NanoDet integration tests failed' }
}
finally {
    $env:FLY_TRACKER_TEST_PYTHON = $oldTestPython
    $env:FLY_TRACKER_NANODET_IMAGES = $oldTestImages
    Pop-Location
}
