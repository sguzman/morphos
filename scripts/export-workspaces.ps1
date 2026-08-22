param(
    [Parameter(Mandatory = $true, Position = 0)]
    [string[]]$Workspace,

    [string]$OutputSubdirectory = "batch",

    [ValidateSet("obj")]
    [string]$Format = "obj",

    [switch]$Overwrite
)

$ErrorActionPreference = "Stop"

foreach ($workspacePath in $Workspace) {
    $workspaceName = Split-Path -Leaf $workspacePath
    $relativePath = Join-Path $OutputSubdirectory "$workspaceName.$Format"

    $args = @(
        "run",
        "-p",
        "geom_app",
        "--bin",
        "morphos",
        "--",
        "export",
        $workspacePath,
        "--format",
        $Format,
        "--destination",
        $relativePath,
        "--json"
    )

    if ($Overwrite) {
        $args += "--overwrite"
    }

    cargo @args
}
