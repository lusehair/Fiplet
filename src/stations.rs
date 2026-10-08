//! Static table of FIP stations: displayed name, Icecast stream, API enum.

/// A FIP webradio.
pub struct Station {
    /// Short name shown in the menu.
    pub name: &'static str,
    /// Icecast stream URL (AAC). The `?id=radiofrance` query is what Radio
    /// France's own web player sends; without it the server prepends a ~30 s
    /// audio advert on every connection (verified: the stream content is
    /// byte-identical between two `id=radiofrance` connections, and shares
    /// nothing with a plain one during its first seconds). It is undocumented
    /// and may stop working, in which case the pre-roll simply comes back.
    pub stream_url: &'static str,
    /// `StationsEnum` value expected by the Radio France GraphQL API.
    pub api_station: &'static str,
    /// Station id in Radio France's unauthenticated `livemeta` API (the one
    /// the radiofrance.fr player uses). `None` when the station is not served
    /// by it (FIP Cultes), in which case only the GraphQL API (key) applies.
    pub livemeta_id: Option<u32>,
}

/// The 13 FIP webradios, in menu display order.
pub const STATIONS: &[Station] = &[
    Station {
        name: "FIP",
        stream_url: "https://icecast.radiofrance.fr/fip-hifi.aac?id=radiofrance",
        api_station: "FIP",
        livemeta_id: Some(7),
    },
    Station {
        name: "FIP Rock",
        stream_url: "https://icecast.radiofrance.fr/fiprock-hifi.aac?id=radiofrance",
        api_station: "FIP_ROCK",
        livemeta_id: Some(64),
    },
    Station {
        name: "FIP Jazz",
        stream_url: "https://icecast.radiofrance.fr/fipjazz-hifi.aac?id=radiofrance",
        api_station: "FIP_JAZZ",
        livemeta_id: Some(65),
    },
    Station {
        name: "FIP Groove",
        stream_url: "https://icecast.radiofrance.fr/fipgroove-hifi.aac?id=radiofrance",
        api_station: "FIP_GROOVE",
        livemeta_id: Some(66),
    },
    Station {
        name: "FIP Electro",
        stream_url: "https://icecast.radiofrance.fr/fipelectro-hifi.aac?id=radiofrance",
        api_station: "FIP_ELECTRO",
        livemeta_id: Some(74),
    },
    Station {
        name: "FIP Metal",
        stream_url: "https://icecast.radiofrance.fr/fipmetal-hifi.aac?id=radiofrance",
        api_station: "FIP_METAL",
        livemeta_id: Some(77),
    },
    Station {
        name: "FIP Hip-Hop",
        stream_url: "https://icecast.radiofrance.fr/fiphiphop-hifi.aac?id=radiofrance",
        api_station: "FIP_HIP_HOP",
        livemeta_id: Some(95),
    },
    Station {
        name: "FIP Pop",
        stream_url: "https://icecast.radiofrance.fr/fippop-hifi.aac?id=radiofrance",
        api_station: "FIP_POP",
        livemeta_id: Some(78),
    },
    Station {
        name: "FIP Cultes",
        stream_url: "https://icecast.radiofrance.fr/fipcultes-hifi.aac?id=radiofrance",
        api_station: "FIP_CULTES",
        livemeta_id: None,
    },
    Station {
        name: "FIP Monde",
        stream_url: "https://icecast.radiofrance.fr/fipworld-hifi.aac?id=radiofrance",
        api_station: "FIP_WORLD",
        livemeta_id: Some(69),
    },
    Station {
        name: "FIP Nouveautés",
        stream_url: "https://icecast.radiofrance.fr/fipnouveautes-hifi.aac?id=radiofrance",
        api_station: "FIP_NOUVEAUTES",
        livemeta_id: Some(70),
    },
    Station {
        name: "FIP Reggae",
        stream_url: "https://icecast.radiofrance.fr/fipreggae-hifi.aac?id=radiofrance",
        api_station: "FIP_REGGAE",
        livemeta_id: Some(71),
    },
    Station {
        name: "FIP Sacré français !",
        stream_url: "https://icecast.radiofrance.fr/fipsacrefrancais-hifi.aac?id=radiofrance",
        api_station: "FIP_SACREFRANCAIS",
        livemeta_id: Some(96),
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_station_uses_the_ad_free_stream_variant() {
        assert_eq!(STATIONS.len(), 13);
        for st in STATIONS {
            assert!(st.stream_url.starts_with("https://icecast.radiofrance.fr/"), "{}", st.name);
            assert!(st.stream_url.ends_with("-hifi.aac?id=radiofrance"), "{}", st.name);
        }
    }

    #[test]
    fn livemeta_ids_are_unique_and_only_cultes_is_missing() {
        let mut ids: Vec<u32> = STATIONS.iter().filter_map(|s| s.livemeta_id).collect();
        assert_eq!(ids.len(), 12);
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), 12);
        let missing: Vec<&str> = STATIONS.iter().filter(|s| s.livemeta_id.is_none()).map(|s| s.name).collect();
        assert_eq!(missing, ["FIP Cultes"]);
    }
}
