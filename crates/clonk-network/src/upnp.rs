use reqwest::{Client, Url};
use std::{
    collections::HashSet,
    future::Future,
    net::{Ipv4Addr, SocketAddr, SocketAddrV4},
    pin::Pin,
    time::Duration,
};
use tokio::{
    net::UdpSocket,
    sync::oneshot,
    task::JoinHandle,
    time::{self, Instant},
};
use xml::reader::{EventReader, XmlEvent};

const SSDP_MULTICAST_ADDRESS: SocketAddrV4 =
    SocketAddrV4::new(Ipv4Addr::new(239, 255, 255, 250), 1900);
const SSDP_DISCOVERY_TIMEOUT: Duration = Duration::from_secs(2);
const HTTP_TIMEOUT: Duration = Duration::from_secs(3);
const MAX_XML_RESPONSE_BYTES: usize = 1024 * 1024;
const PORT_MAPPING_DESCRIPTION: &str = "Clonk Rust";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PortMappingProtocol {
    Tcp,
    Udp,
}

impl PortMappingProtocol {
    fn as_str(self) -> &'static str {
        match self {
            Self::Tcp => "TCP",
            Self::Udp => "UDP",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PortMappingRequest {
    pub(crate) protocol: PortMappingProtocol,
    pub(crate) internal_port: u16,
    pub(crate) external_port: u16,
}

pub(crate) trait ActivePortMappings: Send {
    fn shutdown(self: Box<Self>) -> Pin<Box<dyn Future<Output = ()> + Send>>;
}

pub(crate) trait PortMappingBackend: Send + Sync {
    fn start(&self, requests: &[PortMappingRequest]) -> Box<dyn ActivePortMappings>;
}

#[derive(Debug, Default)]
pub(crate) struct RealPortMappingBackend;

impl PortMappingBackend for RealPortMappingBackend {
    fn start(&self, requests: &[PortMappingRequest]) -> Box<dyn ActivePortMappings> {
        Box::new(RealActivePortMappings::spawn(
            requests.to_vec(),
            UpnpRuntimeConfig::production(),
        ))
    }
}

struct RealActivePortMappings {
    shutdown: Option<oneshot::Sender<()>>,
    task: Option<JoinHandle<()>>,
}

impl RealActivePortMappings {
    fn spawn(requests: Vec<PortMappingRequest>, config: UpnpRuntimeConfig) -> Self {
        let (shutdown, shutdown_rx) = oneshot::channel();
        let task = tokio::runtime::Handle::try_current().ok().map(|runtime| {
            runtime.spawn(run_port_mapping_lifecycle(requests, shutdown_rx, config))
        });

        Self {
            shutdown: Some(shutdown),
            task,
        }
    }

    fn signal_shutdown(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
    }
}

impl ActivePortMappings for RealActivePortMappings {
    fn shutdown(mut self: Box<Self>) -> Pin<Box<dyn Future<Output = ()> + Send>> {
        self.signal_shutdown();
        let task = self.task.take();
        Box::pin(async move {
            if let Some(task) = task {
                let _ = task.await;
            }
        })
    }
}

impl Drop for RealActivePortMappings {
    fn drop(&mut self) {
        self.signal_shutdown();
        // Dropping a Tokio JoinHandle detaches the task. The shutdown signal lets
        // that detached task make a bounded best-effort attempt at cleanup.
    }
}

#[derive(Clone, Copy)]
struct UpnpRuntimeConfig {
    ssdp_target: SocketAddrV4,
    discovery_timeout: Duration,
    http_timeout: Duration,
}

impl UpnpRuntimeConfig {
    const fn production() -> Self {
        Self {
            ssdp_target: SSDP_MULTICAST_ADDRESS,
            discovery_timeout: SSDP_DISCOVERY_TIMEOUT,
            http_timeout: HTTP_TIMEOUT,
        }
    }
}

#[derive(Debug, thiserror::Error)]
enum UpnpError {
    #[error("UPnP I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("UPnP HTTP failed: {0}")]
    Http(#[from] reqwest::Error),
    #[error("UPnP XML failed: {0}")]
    Xml(#[from] xml::reader::Error),
    #[error("UPnP device description is invalid")]
    InvalidDescription,
    #[error("UPnP response is too large")]
    ResponseTooLarge,
}

type UpnpResult<T> = Result<T, UpnpError>;

#[derive(Clone, Debug)]
struct DiscoveredDevice {
    location: Url,
    source: SocketAddr,
}

#[derive(Clone, Debug)]
struct GatewayService {
    control_url: Url,
    service_type: String,
    local_address: Ipv4Addr,
}

#[derive(Clone, Copy, Debug)]
struct EstablishedMapping {
    protocol: PortMappingProtocol,
    external_port: u16,
}

async fn run_port_mapping_lifecycle(
    requests: Vec<PortMappingRequest>,
    mut shutdown: oneshot::Receiver<()>,
    config: UpnpRuntimeConfig,
) {
    let client = match build_http_client(config.http_timeout) {
        Ok(client) => client,
        Err(_) => return,
    };

    // Discovery and description loading have no side effects to undo, so an
    // early shutdown can cancel them immediately.
    let gateway = tokio::select! {
        _ = &mut shutdown => return,
        gateway = locate_gateway(&client, config) => match gateway {
            Ok(gateway) => gateway,
            Err(_) => return,
        },
    };

    let mut mappings = Vec::with_capacity(requests.len());
    let mut stopping = shutdown_requested(&mut shutdown);

    for request in requests {
        if stopping {
            break;
        }

        // Once an Add request is in flight, finish it before observing
        // shutdown. Otherwise the gateway could accept a mapping whose actual
        // external port we never read and therefore cannot delete.
        if let Ok(mapping) = add_port_mapping(&client, &gateway, request).await {
            mappings.push(mapping);
        }
        stopping = shutdown_requested(&mut shutdown);
    }

    if !stopping {
        let _ = shutdown.await;
    }

    for mapping in mappings {
        let _ = delete_port_mapping(&client, &gateway, mapping).await;
    }
}

fn shutdown_requested(shutdown: &mut oneshot::Receiver<()>) -> bool {
    match shutdown.try_recv() {
        Ok(()) | Err(oneshot::error::TryRecvError::Closed) => true,
        Err(oneshot::error::TryRecvError::Empty) => false,
    }
}

fn build_http_client(timeout: Duration) -> UpnpResult<Client> {
    Ok(Client::builder()
        .no_proxy()
        .connect_timeout(timeout)
        .timeout(timeout)
        .redirect(reqwest::redirect::Policy::limited(3))
        .build()?)
}

async fn locate_gateway(client: &Client, config: UpnpRuntimeConfig) -> UpnpResult<GatewayService> {
    let devices = discover_devices(config).await?;
    for device in devices {
        let description = match fetch_xml(client, device.location.clone()).await {
            Ok(description) => description,
            Err(_) => continue,
        };
        let mut gateway = match parse_device_description(&description, &device.location) {
            Ok(gateway) => gateway,
            Err(_) => continue,
        };
        gateway.local_address = match local_ipv4_address_for(device.source).await {
            Ok(address) => address,
            Err(_) => continue,
        };
        return Ok(gateway);
    }
    Err(UpnpError::InvalidDescription)
}

async fn discover_devices(config: UpnpRuntimeConfig) -> UpnpResult<Vec<DiscoveredDevice>> {
    let socket = UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0)).await?;
    socket.set_multicast_ttl_v4(2)?;

    for search_target in [
        "urn:schemas-upnp-org:device:InternetGatewayDevice:2",
        "urn:schemas-upnp-org:device:InternetGatewayDevice:1",
        "upnp:rootdevice",
    ] {
        let request = format!(
            "M-SEARCH * HTTP/1.1\r\nHOST: {}\r\nMAN: \"ssdp:discover\"\r\nMX: 2\r\nST: {}\r\n\r\n",
            config.ssdp_target, search_target
        );
        socket
            .send_to(request.as_bytes(), config.ssdp_target)
            .await?;
    }

    let deadline = Instant::now() + config.discovery_timeout;
    let mut buffer = [0_u8; 8192];
    let mut locations = HashSet::new();
    let mut devices = Vec::new();

    loop {
        let received = match time::timeout_at(deadline, socket.recv_from(&mut buffer)).await {
            Ok(Ok(received)) => received,
            Ok(Err(error)) => return Err(error.into()),
            Err(_) => break,
        };
        let (length, source) = received;
        let Some(location) = parse_ssdp_location(&buffer[..length]) else {
            continue;
        };
        let Ok(location) = Url::parse(&location) else {
            continue;
        };
        if locations.insert(location.as_str().to_owned()) {
            devices.push(DiscoveredDevice { location, source });
        }
    }

    if devices.is_empty() {
        Err(UpnpError::InvalidDescription)
    } else {
        Ok(devices)
    }
}

fn parse_ssdp_location(response: &[u8]) -> Option<String> {
    let response = String::from_utf8_lossy(response);
    if !response
        .lines()
        .next()?
        .trim()
        .to_ascii_uppercase()
        .starts_with("HTTP/1.1 200")
    {
        return None;
    }

    response.lines().skip(1).find_map(|line| {
        let (name, value) = line.split_once(':')?;
        name.trim()
            .eq_ignore_ascii_case("location")
            .then(|| value.trim().to_owned())
    })
}

async fn local_ipv4_address_for(gateway: SocketAddr) -> UpnpResult<Ipv4Addr> {
    let probe = UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0)).await?;
    probe.connect(gateway).await?;
    match probe.local_addr()?.ip() {
        std::net::IpAddr::V4(address) if !address.is_unspecified() => Ok(address),
        _ => Err(UpnpError::InvalidDescription),
    }
}

async fn fetch_xml(client: &Client, url: Url) -> UpnpResult<Vec<u8>> {
    let response = client.get(url).send().await?.error_for_status()?;
    let body = response.bytes().await?;
    if body.len() > MAX_XML_RESPONSE_BYTES {
        return Err(UpnpError::ResponseTooLarge);
    }
    Ok(body.to_vec())
}

#[derive(Default)]
struct DeviceDescriptionService {
    service_type: String,
    control_url: String,
}

fn parse_device_description(xml: &[u8], location: &Url) -> UpnpResult<GatewayService> {
    let parser = EventReader::new(xml);
    let mut url_base = None;
    let mut services = Vec::new();
    let mut current_service: Option<DeviceDescriptionService> = None;
    let mut current_element = None;
    let mut text = String::new();

    for event in parser {
        match event? {
            XmlEvent::StartElement { name, .. } => {
                if name.local_name == "service" {
                    current_service = Some(DeviceDescriptionService::default());
                }
                current_element = Some(name.local_name);
                text.clear();
            }
            XmlEvent::Characters(value) | XmlEvent::CData(value) => {
                text.push_str(&value);
            }
            XmlEvent::EndElement { name } => {
                let value = text.trim();
                match name.local_name.as_str() {
                    "URLBase" if current_service.is_none() => {
                        if !value.is_empty() {
                            url_base = Some(value.to_owned());
                        }
                    }
                    "serviceType" => {
                        if let Some(service) = current_service.as_mut() {
                            service.service_type = value.to_owned();
                        }
                    }
                    "controlURL" => {
                        if let Some(service) = current_service.as_mut() {
                            service.control_url = value.to_owned();
                        }
                    }
                    "service" => {
                        if let Some(service) = current_service.take() {
                            services.push(service);
                        }
                    }
                    _ => {}
                }
                if current_element.as_deref() == Some(name.local_name.as_str()) {
                    current_element = None;
                    text.clear();
                }
            }
            _ => {}
        }
    }

    let (_, service) = services
        .into_iter()
        .filter_map(|service| service_preference(&service.service_type).map(|rank| (rank, service)))
        .min_by_key(|(rank, _)| *rank)
        .ok_or(UpnpError::InvalidDescription)?;

    let base = match url_base {
        Some(base) => Url::parse(&base)
            .or_else(|_| location.join(&base))
            .map_err(|_| UpnpError::InvalidDescription)?,
        None => location.clone(),
    };
    let control_url = Url::parse(&service.control_url)
        .or_else(|_| base.join(&service.control_url))
        .map_err(|_| UpnpError::InvalidDescription)?;

    Ok(GatewayService {
        control_url,
        service_type: service.service_type,
        local_address: Ipv4Addr::UNSPECIFIED,
    })
}

fn service_preference(service_type: &str) -> Option<u8> {
    let (name, version) = service_type.rsplit_once(':')?;
    let version = version.parse::<u16>().ok()?;
    if name.ends_with(":WANIPConnection") {
        match version {
            2.. => Some(0),
            1 => Some(1),
            _ => None,
        }
    } else if name.ends_with(":WANPPPConnection") {
        Some(2)
    } else {
        None
    }
}

async fn add_port_mapping(
    client: &Client,
    gateway: &GatewayService,
    request: PortMappingRequest,
) -> UpnpResult<EstablishedMapping> {
    let requested_external_port = if request.external_port == 0 {
        request.internal_port
    } else {
        request.external_port
    };
    let use_add_any = request.external_port == 0;
    let action = if use_add_any {
        "AddAnyPortMapping"
    } else {
        "AddPortMapping"
    };
    let arguments = format!(
        "<NewRemoteHost></NewRemoteHost>\
         <NewExternalPort>{requested_external_port}</NewExternalPort>\
         <NewProtocol>{}</NewProtocol>\
         <NewInternalPort>{}</NewInternalPort>\
         <NewInternalClient>{}</NewInternalClient>\
         <NewEnabled>1</NewEnabled>\
         <NewPortMappingDescription>{PORT_MAPPING_DESCRIPTION}</NewPortMappingDescription>\
         <NewLeaseDuration>0</NewLeaseDuration>",
        request.protocol.as_str(),
        request.internal_port,
        gateway.local_address,
    );
    let response = soap_request(client, gateway, action, &arguments).await?;
    let external_port = if use_add_any {
        parse_xml_element(&response, "NewReservedPort")?
            .parse::<u16>()
            .map_err(|_| UpnpError::InvalidDescription)?
    } else {
        requested_external_port
    };

    Ok(EstablishedMapping {
        protocol: request.protocol,
        external_port,
    })
}

async fn delete_port_mapping(
    client: &Client,
    gateway: &GatewayService,
    mapping: EstablishedMapping,
) -> UpnpResult<()> {
    let arguments = format!(
        "<NewRemoteHost></NewRemoteHost>\
         <NewExternalPort>{}</NewExternalPort>\
         <NewProtocol>{}</NewProtocol>",
        mapping.external_port,
        mapping.protocol.as_str(),
    );
    soap_request(client, gateway, "DeletePortMapping", &arguments).await?;
    Ok(())
}

async fn soap_request(
    client: &Client,
    gateway: &GatewayService,
    action: &str,
    arguments: &str,
) -> UpnpResult<Vec<u8>> {
    let body = format!(
        "<?xml version=\"1.0\"?>\
         <s:Envelope xmlns:s=\"http://schemas.xmlsoap.org/soap/envelope/\" \
           s:encodingStyle=\"http://schemas.xmlsoap.org/soap/encoding/\">\
         <s:Body><u:{action} xmlns:u=\"{}\">{arguments}</u:{action}></s:Body>\
         </s:Envelope>",
        gateway.service_type,
    );
    let soap_action = format!("\"{}#{action}\"", gateway.service_type);
    let response = client
        .post(gateway.control_url.clone())
        .header(reqwest::header::CONTENT_TYPE, "text/xml; charset=\"utf-8\"")
        .header("SOAPAction", soap_action)
        .body(body)
        .send()
        .await?
        .error_for_status()?;
    let body = response.bytes().await?;
    if body.len() > MAX_XML_RESPONSE_BYTES {
        return Err(UpnpError::ResponseTooLarge);
    }
    Ok(body.to_vec())
}

fn parse_xml_element(xml: &[u8], target: &str) -> UpnpResult<String> {
    let parser = EventReader::new(xml);
    let mut in_target = false;
    let mut value = String::new();
    for event in parser {
        match event? {
            XmlEvent::StartElement { name, .. } if name.local_name == target => {
                in_target = true;
            }
            XmlEvent::Characters(text) | XmlEvent::CData(text) if in_target => {
                value.push_str(&text);
            }
            XmlEvent::EndElement { name } if name.local_name == target => {
                let value = value.trim();
                if value.is_empty() {
                    return Err(UpnpError::InvalidDescription);
                }
                return Ok(value.to_owned());
            }
            _ => {}
        }
    }
    Err(UpnpError::InvalidDescription)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::{TcpListener, TcpStream},
        sync::mpsc,
    };

    #[derive(Debug)]
    struct FakeSoapRequest {
        path: String,
        action: String,
        body: String,
    }

    #[tokio::test]
    async fn cpp_upnp_adds_tcp_then_udp_and_deletes_the_assigned_ports_on_shutdown() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let http_address = listener.local_addr().unwrap();
        let (request_tx, mut request_rx) = mpsc::unbounded_channel();
        let description = Arc::new(format!(
            "<?xml version=\"1.0\"?>\
             <root><URLBase>http://{http_address}/gateway/</URLBase><device><serviceList>\
             <service><serviceType>urn:schemas-upnp-org:service:WANPPPConnection:1</serviceType>\
             <controlURL>wrong-ppp</controlURL></service>\
             <service><serviceType>urn:schemas-upnp-org:service:WANIPConnection:1</serviceType>\
             <controlURL>wrong-v1</controlURL></service>\
             <service><serviceType>urn:schemas-upnp-org:service:WANIPConnection:2</serviceType>\
             <controlURL>control-v2</controlURL></service>\
             </serviceList></device></root>"
        ));
        let http_task = tokio::spawn(run_fake_gateway(listener, description, request_tx, 5));

        let ssdp = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let ssdp_address = match ssdp.local_addr().unwrap() {
            SocketAddr::V4(address) => address,
            SocketAddr::V6(_) => unreachable!(),
        };
        let location = format!("http://{http_address}/root.xml");
        let ssdp_task = tokio::spawn(async move {
            let mut buffer = [0_u8; 2048];
            let (_, peer) = ssdp.recv_from(&mut buffer).await.unwrap();
            let response =
                format!("HTTP/1.1 200 OK\r\nlOcAtIoN: {location}\r\nST: upnp:rootdevice\r\n\r\n");
            ssdp.send_to(response.as_bytes(), peer).await.unwrap();
        });

        let active = RealActivePortMappings::spawn(
            vec![
                PortMappingRequest {
                    protocol: PortMappingProtocol::Tcp,
                    internal_port: 32111,
                    external_port: 0,
                },
                PortMappingRequest {
                    protocol: PortMappingProtocol::Udp,
                    internal_port: 32112,
                    external_port: 0,
                },
            ],
            UpnpRuntimeConfig {
                ssdp_target: ssdp_address,
                discovery_timeout: Duration::from_millis(100),
                http_timeout: Duration::from_secs(1),
            },
        );

        let first_add = recv_fake_request(&mut request_rx).await;
        let second_add = recv_fake_request(&mut request_rx).await;
        assert_eq!(first_add.path, "/gateway/control-v2");
        assert_eq!(first_add.action, "AddAnyPortMapping");
        assert!(first_add.body.contains("<NewProtocol>TCP</NewProtocol>"));
        assert!(first_add
            .body
            .contains("<NewInternalPort>32111</NewInternalPort>"));
        assert!(first_add.body.contains("<NewRemoteHost></NewRemoteHost>"));
        assert!(first_add
            .body
            .contains("<NewPortMappingDescription>Clonk Rust</NewPortMappingDescription>"));
        assert!(first_add
            .body
            .contains("<NewLeaseDuration>0</NewLeaseDuration>"));
        assert_eq!(second_add.action, "AddAnyPortMapping");
        assert!(second_add.body.contains("<NewProtocol>UDP</NewProtocol>"));
        assert!(second_add
            .body
            .contains("<NewInternalPort>32112</NewInternalPort>"));

        Box::new(active).shutdown().await;

        let first_delete = recv_fake_request(&mut request_rx).await;
        let second_delete = recv_fake_request(&mut request_rx).await;
        assert_eq!(first_delete.action, "DeletePortMapping");
        assert!(first_delete.body.contains("<NewProtocol>TCP</NewProtocol>"));
        assert!(first_delete
            .body
            .contains("<NewExternalPort>45111</NewExternalPort>"));
        assert_eq!(second_delete.action, "DeletePortMapping");
        assert!(second_delete
            .body
            .contains("<NewProtocol>UDP</NewProtocol>"));
        assert!(second_delete
            .body
            .contains("<NewExternalPort>45112</NewExternalPort>"));

        ssdp_task.await.unwrap();
        http_task.await.unwrap();
    }

    #[tokio::test]
    async fn upnp_zero_external_port_uses_add_any_on_igd_v1_like_cpp() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let http_address = listener.local_addr().unwrap();
        let (request_tx, mut request_rx) = mpsc::unbounded_channel();
        let description = Arc::new(format!(
            "<?xml version=\"1.0\"?>\
             <root><URLBase>http://{http_address}/gateway/</URLBase><device><serviceList>\
             <service><serviceType>urn:schemas-upnp-org:service:WANIPConnection:1</serviceType>\
             <controlURL>control-v1</controlURL></service>\
             </serviceList></device></root>"
        ));
        let http_task = tokio::spawn(run_fake_gateway(listener, description, request_tx, 6));

        let ssdp = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let ssdp_address = match ssdp.local_addr().unwrap() {
            SocketAddr::V4(address) => address,
            SocketAddr::V6(_) => unreachable!(),
        };
        let location = format!("http://{http_address}/root.xml");
        let ssdp_task = tokio::spawn(async move {
            let mut buffer = [0_u8; 2048];
            let (_, peer) = ssdp.recv_from(&mut buffer).await.unwrap();
            let response =
                format!("HTTP/1.1 200 OK\r\nlOcAtIoN: {location}\r\nST: upnp:rootdevice\r\n\r\n");
            ssdp.send_to(response.as_bytes(), peer).await.unwrap();
        });

        let active = RealActivePortMappings::spawn(
            vec![
                PortMappingRequest {
                    protocol: PortMappingProtocol::Tcp,
                    internal_port: 32999,
                    external_port: 0,
                },
                PortMappingRequest {
                    protocol: PortMappingProtocol::Tcp,
                    internal_port: 32111,
                    external_port: 0,
                },
                PortMappingRequest {
                    protocol: PortMappingProtocol::Udp,
                    internal_port: 32112,
                    external_port: 40123,
                },
            ],
            UpnpRuntimeConfig {
                ssdp_target: ssdp_address,
                discovery_timeout: Duration::from_millis(100),
                http_timeout: Duration::from_secs(1),
            },
        );

        let rejected_add_any = recv_fake_request(&mut request_rx).await;
        let successful_add_any = recv_fake_request(&mut request_rx).await;
        let explicit_add = recv_fake_request(&mut request_rx).await;
        assert_eq!(rejected_add_any.path, "/gateway/control-v1");
        assert_eq!(rejected_add_any.action, "AddAnyPortMapping");
        assert!(rejected_add_any
            .body
            .contains("<NewExternalPort>32999</NewExternalPort>"));
        assert_eq!(successful_add_any.path, "/gateway/control-v1");
        assert_eq!(successful_add_any.action, "AddAnyPortMapping");
        assert!(successful_add_any
            .body
            .contains("<NewExternalPort>32111</NewExternalPort>"));
        assert_eq!(explicit_add.path, "/gateway/control-v1");
        assert_eq!(explicit_add.action, "AddPortMapping");
        assert!(explicit_add
            .body
            .contains("<NewExternalPort>40123</NewExternalPort>"));

        Box::new(active).shutdown().await;

        let add_any_delete = recv_fake_request(&mut request_rx).await;
        let explicit_delete = recv_fake_request(&mut request_rx).await;
        assert_eq!(add_any_delete.action, "DeletePortMapping");
        assert!(add_any_delete
            .body
            .contains("<NewProtocol>TCP</NewProtocol>"));
        assert!(add_any_delete
            .body
            .contains("<NewExternalPort>45111</NewExternalPort>"));
        assert_eq!(explicit_delete.action, "DeletePortMapping");
        assert!(explicit_delete
            .body
            .contains("<NewProtocol>UDP</NewProtocol>"));
        assert!(explicit_delete
            .body
            .contains("<NewExternalPort>40123</NewExternalPort>"));

        ssdp_task.await.unwrap();
        http_task.await.unwrap();
    }

    #[test]
    fn cpp_upnp_v1_and_ppp_services_resolve_relative_control_urls() {
        let location = Url::parse("http://192.0.2.10/devices/root.xml").unwrap();
        let description = br#"
            <root><device><serviceList>
              <service>
                <serviceType>urn:schemas-upnp-org:service:WANPPPConnection:1</serviceType>
                <controlURL>../ppp/control</controlURL>
              </service>
            </serviceList></device></root>
        "#;
        let gateway = parse_device_description(description, &location).unwrap();
        assert_eq!(
            gateway.control_url.as_str(),
            "http://192.0.2.10/ppp/control"
        );
        assert_eq!(
            gateway.service_type,
            "urn:schemas-upnp-org:service:WANPPPConnection:1"
        );
    }

    async fn recv_fake_request(
        requests: &mut mpsc::UnboundedReceiver<FakeSoapRequest>,
    ) -> FakeSoapRequest {
        time::timeout(Duration::from_secs(2), requests.recv())
            .await
            .expect("UPnP request timed out")
            .expect("fake gateway stopped early")
    }

    async fn run_fake_gateway(
        listener: TcpListener,
        description: Arc<String>,
        requests: mpsc::UnboundedSender<FakeSoapRequest>,
        expected_requests: usize,
    ) {
        for _ in 0..expected_requests {
            let (mut stream, _) = listener.accept().await.unwrap();
            let request = read_http_request(&mut stream).await;
            let (status, body) = if request.action.is_empty() {
                ("200 OK", description.as_bytes().to_vec())
            } else {
                let action = request.action.clone();
                let protocol = if request.body.contains("<NewProtocol>TCP</NewProtocol>") {
                    "TCP"
                } else {
                    "UDP"
                };
                let reject_mapping = request
                    .body
                    .contains("<NewInternalPort>32999</NewInternalPort>");
                let service_version = if request.body.contains("WANIPConnection:1") {
                    1
                } else {
                    2
                };
                requests.send(request).unwrap();
                match action.as_str() {
                    "AddAnyPortMapping" if reject_mapping => (
                        "500 Internal Server Error",
                        b"<s:Envelope xmlns:s=\"http://schemas.xmlsoap.org/soap/envelope/\"><s:Body /></s:Envelope>".to_vec(),
                    ),
                    "AddAnyPortMapping" => {
                        let assigned_port = if protocol == "TCP" { 45111 } else { 45112 };
                        (
                            "200 OK",
                            format!(
                                "<s:Envelope xmlns:s=\"http://schemas.xmlsoap.org/soap/envelope/\">\
                                 <s:Body><u:AddAnyPortMappingResponse \
                                 xmlns:u=\"urn:schemas-upnp-org:service:WANIPConnection:{service_version}\">\
                                 <NewReservedPort>{assigned_port}</NewReservedPort>\
                                 </u:AddAnyPortMappingResponse></s:Body></s:Envelope>"
                            )
                            .into_bytes(),
                        )
                    }
                    "AddPortMapping" | "DeletePortMapping" => (
                        "200 OK",
                        b"<s:Envelope xmlns:s=\"http://schemas.xmlsoap.org/soap/envelope/\"><s:Body /></s:Envelope>".to_vec(),
                    ),
                    _ => panic!("unexpected SOAP action {action}"),
                }
            };
            let response = format!(
                "HTTP/1.1 {status}\r\nContent-Type: text/xml\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            stream.write_all(response.as_bytes()).await.unwrap();
            stream.write_all(&body).await.unwrap();
            stream.shutdown().await.unwrap();
        }
    }

    async fn read_http_request(stream: &mut TcpStream) -> FakeSoapRequest {
        let mut bytes = Vec::new();
        let mut buffer = [0_u8; 2048];
        let header_end = loop {
            let read = stream.read(&mut buffer).await.unwrap();
            assert!(read > 0, "HTTP request ended before its headers");
            bytes.extend_from_slice(&buffer[..read]);
            if let Some(position) = find_bytes(&bytes, b"\r\n\r\n") {
                break position + 4;
            }
            assert!(
                bytes.len() < 64 * 1024,
                "HTTP headers are unexpectedly large"
            );
        };
        let headers = String::from_utf8(bytes[..header_end].to_vec()).unwrap();
        let content_length = headers
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse::<usize>().unwrap())
            })
            .unwrap_or(0);
        while bytes.len() < header_end + content_length {
            let read = stream.read(&mut buffer).await.unwrap();
            assert!(read > 0, "HTTP request ended before its body");
            bytes.extend_from_slice(&buffer[..read]);
        }
        let request_line = headers.lines().next().unwrap();
        let path = request_line
            .split_ascii_whitespace()
            .nth(1)
            .unwrap()
            .to_owned();
        let action = headers
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("soapaction").then(|| {
                    value
                        .trim()
                        .trim_matches('"')
                        .rsplit_once('#')
                        .unwrap()
                        .1
                        .to_owned()
                })
            })
            .unwrap_or_default();
        let body =
            String::from_utf8(bytes[header_end..header_end + content_length].to_vec()).unwrap();
        FakeSoapRequest { path, action, body }
    }

    fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
        haystack
            .windows(needle.len())
            .position(|window| window == needle)
    }
}
