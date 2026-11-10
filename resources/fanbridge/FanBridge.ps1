param(
  [Parameter(Mandatory = $true)][string]$DllPath
)

$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
Add-Type -Path $DllPath

$computer = New-Object LibreHardwareMonitor.Hardware.Computer
$computer.IsCpuEnabled = $true
$computer.IsGpuEnabled = $true
$computer.IsMotherboardEnabled = $true
$computer.IsControllerEnabled = $true
$computer.IsStorageEnabled = $false
$computer.IsNetworkEnabled = $false
$computer.IsMemoryEnabled = $false
$computer.Open()

$script:controls = @{}
$script:sensors = @{}

function Update-HardwareTree {
  param($Hardware)
  $Hardware.Update()
  foreach ($child in $Hardware.SubHardware) {
    Update-HardwareTree -Hardware $child
  }
}

function Collect-Hardware {
  param($Hardware)
  Update-HardwareTree -Hardware $Hardware
  foreach ($sensor in $Hardware.Sensors) {
    $id = $sensor.Identifier.ToString()
    switch ($sensor.SensorType) {
      'Control' {
        if ($null -ne $sensor.Control) {
          $script:controls[$id] = @{
            sensor   = $sensor
            hardware = $Hardware
          }
        }
      }
      'Temperature' {
        if ($null -ne $sensor.Value) {
          $script:sensors[$id] = $sensor
        }
      }
    }
  }
  foreach ($child in $Hardware.SubHardware) {
    Collect-Hardware -Hardware $child
  }
}

function Collect-All {
  $script:controls = @{}
  $script:sensors = @{}
  foreach ($hardware in $computer.Hardware) {
    Collect-Hardware -Hardware $hardware
  }
}

function Get-ChannelRecords {
  $items = @()
  foreach ($id in $script:controls.Keys) {
    $entry = $script:controls[$id]
    $sensor = $entry.sensor
    $hardware = $entry.hardware
    $fan = $null
    foreach ($candidate in $hardware.Sensors) {
      if ($candidate.SensorType -eq 'Fan' -and $candidate.Name -eq $sensor.Name) {
        $fan = $candidate
        break
      }
    }
    $rpm = $null
    if ($null -ne $fan) {
      $rpm = $fan.Value
    }
    $items += @{
      id     = $id
      label  = "$($hardware.Name) / $($sensor.Name)"
      maxRpm = 2000
      duty   = $sensor.Value
      rpm    = $rpm
    }
  }
  return $items
}

function Get-SensorRecords {
  $items = @()
  foreach ($id in $script:sensors.Keys) {
    $sensor = $script:sensors[$id]
    $items += @{
      id     = $id
      label  = $sensor.Name
      valueC = $sensor.Value
    }
  }
  return $items
}

function Send-Reply {
  param($Payload)
  $Payload | ConvertTo-Json -Compress -Depth 8
  [Console]::Out.Flush()
}

while ($true) {
  $line = [Console]::In.ReadLine()
  if ($null -eq $line) {
    break
  }
  if ([string]::IsNullOrWhiteSpace($line)) {
    continue
  }

  $request = $null
  try {
    $request = $line | ConvertFrom-Json
    Collect-All

    switch ($request.cmd) {
      'list' {
        Send-Reply @{
          id       = $request.id
          ok       = $true
          channels = @(Get-ChannelRecords)
          sensors  = @(Get-SensorRecords)
        }
      }
      'set' {
        $id = [string]$request.channel
        if (-not $script:controls.ContainsKey($id)) {
          throw "Unknown fan control: $id"
        }
        $percent = [double]$request.percent
        $percent = [Math]::Max(0, [Math]::Min(100, $percent))
        $entry = $script:controls[$id]
        $entry.sensor.Control.SetSoftware([float]$percent)
        $entry.hardware.Update()
        Send-Reply @{ id = $request.id; ok = $true }
      }
      'restore' {
        foreach ($entry in $script:controls.Values) {
          try {
            $entry.sensor.Control.SetDefault()
            $entry.hardware.Update()
          } catch {
          }
        }
        Send-Reply @{ id = $request.id; ok = $true }
      }
      default {
        throw "Unknown command: $($request.cmd)"
      }
    }
  } catch {
    $id = if ($null -ne $request -and $null -ne $request.id) { $request.id } else { 0 }
    Send-Reply @{ id = $id; ok = $false; error = $_.Exception.Message }
  }
}

$computer.Close()
