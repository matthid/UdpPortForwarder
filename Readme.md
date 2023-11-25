# Simple UdpPortForwarder

Download and extract the binaries to "C:\Tools\UdpPortForwarder.exe" and use the following script.
(You can change the download path in the script)

```powershell
$distroName       = "ubuntu"
$WSLip = wsl -d $distroName -e sh -c "ip addr show eth0 | grep 'inet\b' | awk '{print `$2}' | cut -d/ -f1"

#[Ports]

#All the ports you want to forward separated by coma
$UDPports=@(7777,15000,15777);

#[Static ip]
#You can change the addr to your ip config to listen to a specific address
$addr='0.0.0.0';
$ports_a = $UDPports -join ",";

#Remove Firewall Exception Rules
iex "Remove-NetFireWallRule -DisplayName 'WSL 2 Ports' ";

#adding Exception Rules for inbound and outbound Rules
iex "New-NetFireWallRule -DisplayName 'WSL 2 Ports' -Direction Outbound -LocalPort $ports_a -Action Allow -Protocol UDP";
iex "New-NetFireWallRule -DisplayName 'WSL 2 Ports' -Direction Inbound -LocalPort $ports_a -Action Allow -Protocol UDP";

& "C:\Tools\UdpPortForwarder.exe" $WSLip @UDPports
```

As an alternative you can clone the project, install cargo and use the following
(to start the forwarding directly from the code)

```
cd C:\Projects\UdpPortForwarder
& "C:\Users\<user>\.cargo\bin\cargo.exe" run --release -- $WSLip @UDPports
```