//! Static table of FIP stations: displayed name, Icecast stream, API enum.

/// A FIP webradio.
pub struct Station {
    /// Short name shown in the menu.
    pub name: &'static str,
    /// Icecast stream URL (AAC).
    pub stream_url: &'static str,
    /// `StationsEnum` value expected by the Radio France GraphQL API.
    pub api_station: &'static str,
}

/// The 13 FIP webradios, in menu display order.
pub const STATIONS: &[Station] = &[
    Station {
        name: "FIP",
        stream_url: "https://icecast.radiofrance.fr/fip-hifi.aac",
        api_station: "FIP",
    },
    Station {
        name: "FIP Rock",
        stream_url: "https://icecast.radiofrance.fr/fiprock-hifi.aac",
        api_station: "FIP_ROCK",
    },
    Station {
        name: "FIP Jazz",
        stream_url: "https://icecast.radiofrance.fr/fipjazz-hifi.aac",
        api_station: "FIP_JAZZ",
    },
    Station {
        name: "FIP Groove",
        stream_url: "https://icecast.radiofrance.fr/fipgroove-hifi.aac",
        api_station: "FIP_GROOVE",
    },
    Station {
        name: "FIP Electro",
        stream_url: "https://icecast.radiofrance.fr/fipelectro-hifi.aac",
        api_station: "FIP_ELECTRO",
    },
    Station {
        name: "FIP Metal",
        stream_url: "https://icecast.radiofrance.fr/fipmetal-hifi.aac",
        api_station: "FIP_METAL",
    },
    Station {
        name: "FIP Hip-Hop",
        stream_url: "https://icecast.radiofrance.fr/fiphiphop-hifi.aac",
        api_station: "FIP_HIP_HOP",
    },
    Station {
        name: "FIP Pop",
        stream_url: "https://icecast.radiofrance.fr/fippop-hifi.aac",
        api_station: "FIP_POP",
    },
    Station {
        name: "FIP Cultes",
        stream_url: "https://icecast.radiofrance.fr/fipcultes-hifi.aac",
        api_station: "FIP_CULTES",
    },
    Station {
        name: "FIP Monde",
        stream_url: "https://icecast.radiofrance.fr/fipworld-hifi.aac",
        api_station: "FIP_WORLD",
    },
    Station {
        name: "FIP Nouveautés",
        stream_url: "https://icecast.radiofrance.fr/fipnouveautes-hifi.aac",
        api_station: "FIP_NOUVEAUTES",
    },
    Station {
        name: "FIP Reggae",
        stream_url: "https://icecast.radiofrance.fr/fipreggae-hifi.aac",
        api_station: "FIP_REGGAE",
    },
    Station {
        name: "FIP Sacré français !",
        stream_url: "https://icecast.radiofrance.fr/fipsacrefrancais-hifi.aac",
        api_station: "FIP_SACREFRANCAIS",
    },
];
