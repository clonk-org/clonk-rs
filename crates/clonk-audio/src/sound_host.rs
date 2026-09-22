//! The cpal host audio devices come from, and which of its endpoints a
//! player is offered.
//!
//! On Linux `cpal::default_host` prefers the PulseAudio host whenever a
//! PulseAudio or pipewire-pulse server is running, and falls back to ALSA.

/// The cpal host one device worker lists and opens devices through.
///
/// A sound-server host holds a connection, so a worker that polls for
/// devices keeps it rather than reconnecting on every poll. It is dropped
/// after a failure, because a server that restarted leaves the old
/// connection unusable, and reopened on next use.
#[cfg(feature = "cpal")]
#[derive(Default)]
pub(crate) struct SoundHost(Option<cpal::Host>);

#[cfg(feature = "cpal")]
impl SoundHost {
    pub(crate) fn get(&mut self) -> &cpal::Host {
        self.0.get_or_insert_with(cpal::default_host)
    }

    pub(crate) fn reset(&mut self) {
        self.0 = None;
    }

    /// Runs `query` against the host, dropping the host if it fails.
    pub(crate) fn with<T, E>(
        &mut self,
        query: impl FnOnce(&cpal::Host) -> Result<T, E>,
    ) -> Result<T, E> {
        let result = query(self.get());
        if result.is_err() {
            self.reset();
        }
        result
    }
}

/// One endpoint as its host reports it: the persisted `<host>:<device>` ID
/// and the label the host gives it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct HostEndpoint {
    pub(crate) id: String,
    pub(crate) name: String,
}

/// Reads the ID and label of each device, skipping any that disappears
/// while being read.
#[cfg(feature = "cpal")]
pub(crate) fn host_endpoints(devices: impl Iterator<Item = cpal::Device>) -> Vec<HostEndpoint> {
    use cpal::traits::DeviceTrait;

    devices
        .filter_map(|device| {
            let id = device.id().map_err(|error| {
                tracing::warn!(%error, "audio device disappeared while reading its ID");
            });
            let description = device.description().map_err(|error| {
                tracing::warn!(%error, "audio device disappeared while reading its description");
            });
            id.ok()
                .zip(description.ok())
                .map(|(id, description)| HostEndpoint {
                    id: id.to_string(),
                    name: description.name().to_string(),
                })
        })
        .collect()
}

/// The input endpoints worth offering as a microphone.
///
/// A sound server gives every output a monitor source that captures what
/// that output plays. PulseAudio and pipewire-pulse both name it
/// `<sink>.monitor`.
pub(crate) fn offered_inputs(inputs: Vec<HostEndpoint>) -> Vec<HostEndpoint> {
    offered_alsa_pcms(inputs)
        .into_iter()
        .filter(|input| {
            input
                .id
                .strip_prefix("pulseaudio:")
                .is_none_or(|source| !source.ends_with(".monitor"))
        })
        .collect()
}

/// The output endpoints worth offering as a speaker.
pub(crate) fn offered_outputs(outputs: Vec<HostEndpoint>) -> Vec<HostEndpoint> {
    offered_alsa_pcms(outputs)
}

/// ALSA PCMs that address one card, most preferred first: the card's
/// default, then the PCMs that reach a device the default does not, then
/// the converting PCM cpal adds for every physical device.
const ALSA_CARD_PCMS: [&str; 5] = ["sysdefault", "front", "hdmi", "iec958", "plughw"];

/// ALSA lists every PCM its configuration defines, not the devices a player
/// knows. Per card, keep one PCM for each device label, the most preferred
/// of [`ALSA_CARD_PCMS`]. The rest (`null`, the system `default` that
/// "System default" already stands for, rate and mixing plugins,
/// sound-server bridges, raw `hw`, `surround*` and `usbstream`) are routes
/// to a device, not devices. Endpoints of other hosts pass through.
///
/// cpal addresses the `plughw` PCM it adds by card index, which a hint
/// never does, so it cannot be matched to a hint's card. It is kept only
/// for a device no hint names, such as a card ALSA has no configuration for.
fn offered_alsa_pcms(endpoints: Vec<HostEndpoint>) -> Vec<HostEndpoint> {
    let pcms = endpoints
        .iter()
        .enumerate()
        .map(|(order, endpoint)| AlsaCardPcm::parse(endpoint, order))
        .collect::<Vec<_>>();
    let keep = endpoints
        .iter()
        .zip(&pcms)
        .map(|(endpoint, pcm)| match pcm {
            _ if !endpoint.id.starts_with("alsa:") => true,
            None => false,
            Some(pcm) => !pcms.iter().flatten().any(|other| other.supersedes(pcm)),
        })
        .collect::<Vec<_>>();
    endpoints
        .into_iter()
        .zip(keep)
        .filter_map(|(endpoint, keep)| keep.then_some(endpoint))
        .collect()
}

/// An ALSA PCM of one of the [`ALSA_CARD_PCMS`] kinds.
struct AlsaCardPcm<'a> {
    /// Position of its kind in [`ALSA_CARD_PCMS`].
    rank: usize,
    /// Position in the host's enumeration, which breaks ties in `rank`.
    order: usize,
    card: &'a str,
    label: &'a str,
}

impl<'a> AlsaCardPcm<'a> {
    fn parse(endpoint: &'a HostEndpoint, order: usize) -> Option<Self> {
        let (kind, arguments) = endpoint.id.strip_prefix("alsa:")?.split_once(':')?;
        let rank = ALSA_CARD_PCMS.iter().position(|known| *known == kind)?;
        let card = arguments
            .split(',')
            .find_map(|argument| argument.strip_prefix("CARD="))?;
        Some(Self {
            rank,
            order,
            card,
            label: &endpoint.name,
        })
    }

    /// Whether this PCM reaches the device `other` does and is preferred.
    fn supersedes(&self, other: &Self) -> bool {
        self.label == other.label
            && if self.card == other.card {
                (self.rank, self.order) < (other.rank, other.order)
            } else {
                is_card_index(other.card) && !is_card_index(self.card)
            }
    }
}

fn is_card_index(card: &str) -> bool {
    card.bytes().all(|byte| byte.is_ascii_digit())
}

/// Whether a saved input or output device ID belongs to a host other than
/// the one devices are listed and opened through.
///
/// No listed device can match such an ID, so it means the system default
/// rather than a device that is unplugged. The case this exists for is an
/// ALSA ID saved before the PulseAudio host was built in. An ID that names
/// no host built for this platform, corrupt or saved on another operating
/// system, stays an exact selection.
///
/// The host compared against is the one `cpal::default_host` tries first.
/// If that host is found but fails to connect, cpal falls back to the next
/// one; IDs are still judged against the host it found.
pub fn saved_device_follows_system_default(id: &str) -> bool {
    #[cfg(feature = "cpal")]
    {
        cpal::available_hosts()
            .first()
            .is_some_and(|active| names_other_host(id, *active))
    }
    #[cfg(not(feature = "cpal"))]
    {
        let _ = id;
        false
    }
}

#[cfg(feature = "cpal")]
fn names_other_host(id: &str, active: cpal::HostId) -> bool {
    id.split_once(':')
        .and_then(|(host, _)| host.parse::<cpal::HostId>().ok())
        .is_some_and(|host| host != active)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn endpoint(id: &str, name: &str) -> HostEndpoint {
        HostEndpoint {
            id: id.into(),
            name: name.into(),
        }
    }

    #[test]
    fn microphones_leave_out_the_monitor_of_each_output() {
        let webcam = endpoint(
            "pulseaudio:alsa_input.usb-046d_HD_Pro_Webcam_C920_8734B79F-02.analog-stereo",
            "C920 PRO HD Webcam Analog Stereo",
        );
        let inputs = vec![
            endpoint(
                "pulseaudio:alsa_output.pci-0000_75_00.6.analog-stereo.monitor",
                "Monitor of Ryzen HD Audio Controller Analog Stereo",
            ),
            webcam.clone(),
        ];
        assert_eq!(offered_inputs(inputs), vec![webcam]);
    }

    /// Every hint cpal 0.18's ALSA host enumerated for capture on a desktop
    /// with a Blue Yeti, a C920 webcam, onboard ALC1220 audio and an
    /// AVerMedia capture card, in enumeration order.
    fn alsa_capture_endpoints() -> Vec<HostEndpoint> {
        [
            (
                "alsa:null",
                "Discard all samples (playback) or generate zero samples (capture)",
            ),
            ("alsa:sysdefault", "Default Audio Device"),
            (
                "alsa:lavrate",
                "Rate Converter Plugin Using Libav/FFmpeg Library",
            ),
            (
                "alsa:samplerate",
                "Rate Converter Plugin Using Samplerate Library",
            ),
            (
                "alsa:speexrate",
                "Rate Converter Plugin Using Speex Resampler",
            ),
            ("alsa:jack", "JACK Audio Connection Kit"),
            ("alsa:oss", "Open Sound System"),
            ("alsa:pipewire", "PipeWire Sound Server"),
            ("alsa:pulse", "PulseAudio Sound Server"),
            (
                "alsa:speex",
                "Plugin using Speex DSP (resample, agc, denoise, echo, dereverb)",
            ),
            ("alsa:upmix", "Plugin for channel upmix (4,6,8)"),
            (
                "alsa:vdownmix",
                "Plugin for channel downmix (stereo) with a simple spacialization",
            ),
            (
                "alsa:default",
                "Default ALSA Output (currently PipeWire Media Server)",
            ),
            (
                "alsa:sysdefault:CARD=Microphones",
                "Blue Microphones, USB Audio",
            ),
            (
                "alsa:front:CARD=Microphones,DEV=0",
                "Blue Microphones, USB Audio",
            ),
            ("alsa:usbstream:CARD=Microphones", "Blue Microphones"),
            ("alsa:usbstream:CARD=NVidia", "HDA NVidia"),
            ("alsa:usbstream:CARD=Generic", "HD-Audio Generic"),
            (
                "alsa:sysdefault:CARD=Generic_1",
                "HD-Audio Generic, ALC1220 Analog",
            ),
            (
                "alsa:front:CARD=Generic_1,DEV=0",
                "HD-Audio Generic, ALC1220 Analog",
            ),
            ("alsa:usbstream:CARD=Generic_1", "HD-Audio Generic"),
            ("alsa:sysdefault:CARD=C920", "HD Pro Webcam C920, USB Audio"),
            (
                "alsa:front:CARD=C920,DEV=0",
                "HD Pro Webcam C920, USB Audio",
            ),
            ("alsa:usbstream:CARD=C920", "HD Pro Webcam C920"),
            (
                "alsa:sysdefault:CARD=Device",
                "AVerMedia USB Device, USB Audio",
            ),
            (
                "alsa:front:CARD=Device,DEV=0",
                "AVerMedia USB Device, USB Audio",
            ),
            ("alsa:usbstream:CARD=Device", "AVerMedia USB Device"),
            ("alsa:hw:CARD=0,DEV=0", "Blue Microphones, USB Audio"),
            ("alsa:plughw:CARD=0,DEV=0", "Blue Microphones, USB Audio"),
            ("alsa:hw:CARD=3,DEV=0", "HD-Audio Generic, ALC1220 Analog"),
            (
                "alsa:plughw:CARD=3,DEV=0",
                "HD-Audio Generic, ALC1220 Analog",
            ),
            ("alsa:hw:CARD=4,DEV=0", "HD Pro Webcam C920, USB Audio"),
            ("alsa:plughw:CARD=4,DEV=0", "HD Pro Webcam C920, USB Audio"),
            ("alsa:hw:CARD=5,DEV=0", "AVerMedia USB Device, USB Audio"),
            (
                "alsa:plughw:CARD=5,DEV=0",
                "AVerMedia USB Device, USB Audio",
            ),
        ]
        .into_iter()
        .map(|(id, name)| endpoint(id, name))
        .collect()
    }

    #[test]
    fn alsa_microphones_are_one_entry_per_card() {
        assert_eq!(
            offered_inputs(alsa_capture_endpoints()),
            [
                (
                    "alsa:sysdefault:CARD=Microphones",
                    "Blue Microphones, USB Audio"
                ),
                (
                    "alsa:sysdefault:CARD=Generic_1",
                    "HD-Audio Generic, ALC1220 Analog",
                ),
                ("alsa:sysdefault:CARD=C920", "HD Pro Webcam C920, USB Audio"),
                (
                    "alsa:sysdefault:CARD=Device",
                    "AVerMedia USB Device, USB Audio",
                ),
            ]
            .map(|(id, name)| endpoint(id, name))
        );
    }

    /// The playback half of the same enumeration, minus the card-less
    /// plugins [`alsa_capture_endpoints`] already covers.
    fn alsa_playback_endpoints() -> Vec<HostEndpoint> {
        let blue = "Blue Microphones, USB Audio";
        let analog = "HD-Audio Generic, ALC1220 Analog";
        [
            (
                "alsa:null",
                "Discard all samples (playback) or generate zero samples (capture)",
            ),
            (
                "alsa:default",
                "Default ALSA Output (currently PipeWire Media Server)",
            ),
            ("alsa:sysdefault:CARD=Microphones", blue),
            ("alsa:front:CARD=Microphones,DEV=0", blue),
            ("alsa:surround21:CARD=Microphones,DEV=0", blue),
            ("alsa:surround51:CARD=Microphones,DEV=0", blue),
            ("alsa:iec958:CARD=Microphones,DEV=0", blue),
            ("alsa:usbstream:CARD=Microphones", "Blue Microphones"),
            ("alsa:hdmi:CARD=NVidia,DEV=0", "HDA NVidia, XG240R SERIES"),
            ("alsa:hdmi:CARD=NVidia,DEV=1", "HDA NVidia, HDMI 1"),
            ("alsa:usbstream:CARD=NVidia", "HDA NVidia"),
            ("alsa:hdmi:CARD=Generic,DEV=0", "HD-Audio Generic, HDMI 0"),
            ("alsa:usbstream:CARD=Generic", "HD-Audio Generic"),
            ("alsa:sysdefault:CARD=Generic_1", analog),
            ("alsa:front:CARD=Generic_1,DEV=0", analog),
            ("alsa:surround71:CARD=Generic_1,DEV=0", analog),
            (
                "alsa:iec958:CARD=Generic_1,DEV=0",
                "HD-Audio Generic, ALC1220 Digital",
            ),
            ("alsa:usbstream:CARD=Generic_1", "HD-Audio Generic"),
            ("alsa:hw:CARD=0,DEV=0", blue),
            ("alsa:plughw:CARD=0,DEV=0", blue),
            ("alsa:hw:CARD=1,DEV=3", "HDA NVidia, XG240R SERIES"),
            ("alsa:plughw:CARD=1,DEV=3", "HDA NVidia, XG240R SERIES"),
            ("alsa:hw:CARD=1,DEV=7", "HDA NVidia, HDMI 1"),
            ("alsa:plughw:CARD=1,DEV=7", "HDA NVidia, HDMI 1"),
            ("alsa:hw:CARD=2,DEV=3", "HD-Audio Generic, HDMI 0"),
            ("alsa:plughw:CARD=2,DEV=3", "HD-Audio Generic, HDMI 0"),
            ("alsa:hw:CARD=3,DEV=0", analog),
            ("alsa:plughw:CARD=3,DEV=0", analog),
            ("alsa:hw:CARD=3,DEV=1", "HD-Audio Generic, ALC1220 Digital"),
            (
                "alsa:plughw:CARD=3,DEV=1",
                "HD-Audio Generic, ALC1220 Digital",
            ),
        ]
        .into_iter()
        .map(|(id, name)| endpoint(id, name))
        .collect()
    }

    #[test]
    fn alsa_outputs_keep_each_card_output_a_card_default_does_not_cover() {
        assert_eq!(
            offered_outputs(alsa_playback_endpoints()),
            [
                (
                    "alsa:sysdefault:CARD=Microphones",
                    "Blue Microphones, USB Audio"
                ),
                ("alsa:hdmi:CARD=NVidia,DEV=0", "HDA NVidia, XG240R SERIES"),
                ("alsa:hdmi:CARD=NVidia,DEV=1", "HDA NVidia, HDMI 1"),
                ("alsa:hdmi:CARD=Generic,DEV=0", "HD-Audio Generic, HDMI 0"),
                (
                    "alsa:sysdefault:CARD=Generic_1",
                    "HD-Audio Generic, ALC1220 Analog",
                ),
                (
                    "alsa:iec958:CARD=Generic_1,DEV=0",
                    "HD-Audio Generic, ALC1220 Digital",
                ),
            ]
            .map(|(id, name)| endpoint(id, name))
        );
    }

    #[test]
    fn alsa_card_without_configured_hints_is_offered_through_its_converting_pcm() {
        let usb = "USB Audio Device, USB Audio";
        let endpoints = [
            ("alsa:null", "Discard all samples"),
            ("alsa:default", "Default Audio Device"),
            ("alsa:hw:CARD=0,DEV=0", usb),
            ("alsa:plughw:CARD=0,DEV=0", usb),
            ("alsa:hw:CARD=1,DEV=0", usb),
            ("alsa:plughw:CARD=1,DEV=0", usb),
        ]
        .map(|(id, name)| endpoint(id, name));
        assert_eq!(
            offered_inputs(endpoints.to_vec()),
            [endpoints[3].clone(), endpoints[5].clone()]
        );
    }

    #[test]
    fn alsa_offers_each_of_two_identical_cards() {
        let webcam = "HD Pro Webcam C920, USB Audio";
        let endpoints = [
            ("alsa:sysdefault:CARD=C920", webcam),
            ("alsa:front:CARD=C920,DEV=0", webcam),
            ("alsa:sysdefault:CARD=C920_1", webcam),
            ("alsa:front:CARD=C920_1,DEV=0", webcam),
            ("alsa:plughw:CARD=4,DEV=0", webcam),
            ("alsa:plughw:CARD=5,DEV=0", webcam),
        ]
        .map(|(id, name)| endpoint(id, name));
        assert_eq!(
            offered_inputs(endpoints.to_vec()),
            [endpoints[0].clone(), endpoints[2].clone()]
        );
    }

    #[cfg(all(feature = "cpal", target_os = "linux"))]
    #[test]
    fn saved_device_of_the_host_before_a_sound_server_follows_the_default() {
        let active = cpal::HostId::PulseAudio;
        assert!(names_other_host("alsa:plughw:CARD=4,DEV=0", active));
        assert!(!names_other_host(
            "pulseaudio:alsa_input.usb-046d_HD_Pro_Webcam_C920_8734B79F-02.analog-stereo",
            active
        ));
        assert!(names_other_host(
            "pulseaudio:alsa_output.pci-0000_75_00.6.analog-stereo",
            cpal::HostId::Alsa
        ));
    }

    #[cfg(all(feature = "cpal", target_os = "linux"))]
    #[test]
    fn saved_device_naming_no_host_of_this_platform_stays_selected() {
        for id in [
            "corrupt persisted identity",
            "coreaudio:old-device",
            "wasapi:{0.0.1.00000000}.{guid}",
        ] {
            assert!(!names_other_host(id, cpal::HostId::PulseAudio), "{id}");
        }
    }
}
