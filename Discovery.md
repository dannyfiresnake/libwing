# Behringer Wing Digital Mixer Discovery Protocol

This simple protocol allows a client on the network to discover Wing devices on
the network. **libwing** implements this discovery protocol.

`WingConsole::scan()` probes each active IPv4 interface with a broadcast address,
excluding loopback and point-to-point interfaces. It sends to each interface's
subnet broadcast address; on Apple platforms the socket is also explicitly bound
to that interface so VPN default routes cannot capture discovery. An interface
failure does not prevent discovery on the others. Replies are deduplicated by
IP address and serial number, and all interfaces share a five-second deadline
(`scan(true)` returns as soon as a mixer is found).

## VPNs and multiple network interfaces

Earlier versions sent one broadcast to `255.255.255.255` using the system's
default route. A VPN can claim that route even when it allows connections to
specific LAN addresses. In that case, connecting to a known mixer IP can work
while discovery fails.

Starting in 1.0.6, each eligible interface has a separate nonblocking socket,
bound to its local IPv4 address, which sends `WING?` to its subnet broadcast
address. On Apple platforms, `IP_BOUND_IF` additionally pins the socket to the
interface; binding only the local address is insufficient to override the VPN
broadcast route there. Other platforms use the local address and subnet
broadcast route. This change has been tested with live hardware on macOS.

All probes are sent before collecting replies. A failed interface does not
abort probes on other interfaces. If every eligible probe fails, the scan
returns an I/O error; if there are no eligible interfaces, it returns an empty
list. Valid replies contain a `WING` prefix, an IPv4 address, and the remaining
device fields. Repeated replies with the same IP address and serial number
appear only once in the results.

The scan does not relay broadcasts through routers or establish a tunnel to a
remote mixer. The subsequent TCP connection still follows normal OS routing.
If discovery works but connection fails, check that the VPN permits LAN access
and that TCP port 2222 is reachable. A known mixer IP can be supplied to
`WingConsole::connect(Some("192.168.1.100"))` to bypass discovery.

## Wire protocol

To discover a Wing device on your network, send a 5 byte UDP packet containing
the following bytes to your network broadcast IP on UDP port 2222.

```
0x57, 0x49, 0x4E, 0x47, 0x3F       // ASCII for W, I, N, G, ?
```

Your Wing unit(s) will respond with a UDP packet containing a commma-separated
list of fields. The fields are as follows:

```
WING,ip,name,model,serial,firmware
```

For example, my Wing Compact running firmware 3.0.5 looks like this (I've
altered the identifying information for privacy reasons):

```
WING,192.168.1.19,WING-PP-93892216,wing-compact,01XXXUX06X3AEX,3.0.5-0-g0c2b9d4a:release
```

Other models seem to be:

- "wing" (possibly the full Wing in white)
- "wing-rack" (the rackmount version)
- "wing-bk" (a black version of the full Wing console)
- "ngc-full" (the full Wing console in white)

I can only confirm the "wing-compact" model, as that is the only one I have
access to. If someone would like to report the model for any others, please do
so with a pull request updating this doc.

##### Broadcast IP

You can compute the broadcast IP by taking the bitwise OR of the IP and the
bitwise NOT of the subnet mask (broadcast = ip | ~subnet). For example, if you
are 192.168.1.23 / 255.255.255.0, then your broadcast would be 192.168.1.255.

You can also send it to 255.255.255.255 as your router will most likely block
your packet from leaving your local network.

Note: when sending a packet using most operating system's networking APIs, you
will need to set the flag on the socket to allow sending to broadcast IPs.
