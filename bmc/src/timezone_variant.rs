// Copyright (C) 2025  Braiins Systems s.r.o.

use crate::time::Timezone;

/// This array was taken from openwrt supported timezone variants
/// https://github.com/openwrt/luci/blob/master/modules/luci-lua-runtime/luasrc/sys/zoneinfo/tzdata.lua
pub(crate) const TIMEZONE_VARIANTS: [Timezone; 449] = [
    Timezone {
        iana: "Africa/Abidjan",
        posix: "GMT0",
    },
    Timezone {
        iana: "Africa/Accra",
        posix: "GMT0",
    },
    Timezone {
        iana: "Africa/Addis Ababa",
        posix: "EAT-3",
    },
    Timezone {
        iana: "Africa/Algiers",
        posix: "CET-1",
    },
    Timezone {
        iana: "Africa/Asmara",
        posix: "EAT-3",
    },
    Timezone {
        iana: "Africa/Bamako",
        posix: "GMT0",
    },
    Timezone {
        iana: "Africa/Bangui",
        posix: "WAT-1",
    },
    Timezone {
        iana: "Africa/Banjul",
        posix: "GMT0",
    },
    Timezone {
        iana: "Africa/Bissau",
        posix: "GMT0",
    },
    Timezone {
        iana: "Africa/Blantyre",
        posix: "CAT-2",
    },
    Timezone {
        iana: "Africa/Brazzaville",
        posix: "WAT-1",
    },
    Timezone {
        iana: "Africa/Bujumbura",
        posix: "CAT-2",
    },
    Timezone {
        iana: "Africa/Cairo",
        posix: "EET-2",
    },
    Timezone {
        iana: "Africa/Casablanca",
        posix: "<+01>-1",
    },
    Timezone {
        iana: "Africa/Ceuta",
        posix: "CET-1CEST,M3.5.0,M10.5.0/3",
    },
    Timezone {
        iana: "Africa/Conakry",
        posix: "GMT0",
    },
    Timezone {
        iana: "Africa/Dakar",
        posix: "GMT0",
    },
    Timezone {
        iana: "Africa/Dar es Salaam",
        posix: "EAT-3",
    },
    Timezone {
        iana: "Africa/Djibouti",
        posix: "EAT-3",
    },
    Timezone {
        iana: "Africa/Douala",
        posix: "WAT-1",
    },
    Timezone {
        iana: "Africa/El Aaiun",
        posix: "<+01>-1",
    },
    Timezone {
        iana: "Africa/Freetown",
        posix: "GMT0",
    },
    Timezone {
        iana: "Africa/Gaborone",
        posix: "CAT-2",
    },
    Timezone {
        iana: "Africa/Harare",
        posix: "CAT-2",
    },
    Timezone {
        iana: "Africa/Johannesburg",
        posix: "SAST-2",
    },
    Timezone {
        iana: "Africa/Juba",
        posix: "CAT-2",
    },
    Timezone {
        iana: "Africa/Kampala",
        posix: "EAT-3",
    },
    Timezone {
        iana: "Africa/Khartoum",
        posix: "CAT-2",
    },
    Timezone {
        iana: "Africa/Kigali",
        posix: "CAT-2",
    },
    Timezone {
        iana: "Africa/Kinshasa",
        posix: "WAT-1",
    },
    Timezone {
        iana: "Africa/Lagos",
        posix: "WAT-1",
    },
    Timezone {
        iana: "Africa/Libreville",
        posix: "WAT-1",
    },
    Timezone {
        iana: "Africa/Lome",
        posix: "GMT0",
    },
    Timezone {
        iana: "Africa/Luanda",
        posix: "WAT-1",
    },
    Timezone {
        iana: "Africa/Lubumbashi",
        posix: "CAT-2",
    },
    Timezone {
        iana: "Africa/Lusaka",
        posix: "CAT-2",
    },
    Timezone {
        iana: "Africa/Malabo",
        posix: "WAT-1",
    },
    Timezone {
        iana: "Africa/Maputo",
        posix: "CAT-2",
    },
    Timezone {
        iana: "Africa/Maseru",
        posix: "SAST-2",
    },
    Timezone {
        iana: "Africa/Mbabane",
        posix: "SAST-2",
    },
    Timezone {
        iana: "Africa/Mogadishu",
        posix: "EAT-3",
    },
    Timezone {
        iana: "Africa/Monrovia",
        posix: "GMT0",
    },
    Timezone {
        iana: "Africa/Nairobi",
        posix: "EAT-3",
    },
    Timezone {
        iana: "Africa/Ndjamena",
        posix: "WAT-1",
    },
    Timezone {
        iana: "Africa/Niamey",
        posix: "WAT-1",
    },
    Timezone {
        iana: "Africa/Nouakchott",
        posix: "GMT0",
    },
    Timezone {
        iana: "Africa/Ouagadougou",
        posix: "GMT0",
    },
    Timezone {
        iana: "Africa/Porto-Novo",
        posix: "WAT-1",
    },
    Timezone {
        iana: "Africa/Sao Tome",
        posix: "GMT0",
    },
    Timezone {
        iana: "Africa/Tripoli",
        posix: "EET-2",
    },
    Timezone {
        iana: "Africa/Tunis",
        posix: "CET-1",
    },
    Timezone {
        iana: "Africa/Windhoek",
        posix: "CAT-2",
    },
    Timezone {
        iana: "America/Adak",
        posix: "HST10HDT,M3.2.0,M11.1.0",
    },
    Timezone {
        iana: "America/Anchorage",
        posix: "AKST9AKDT,M3.2.0,M11.1.0",
    },
    Timezone {
        iana: "America/Anguilla",
        posix: "AST4",
    },
    Timezone {
        iana: "America/Antigua",
        posix: "AST4",
    },
    Timezone {
        iana: "America/Araguaina",
        posix: "<-03>3",
    },
    Timezone {
        iana: "America/Argentina/Buenos Aires",
        posix: "<-03>3",
    },
    Timezone {
        iana: "America/Argentina/Catamarca",
        posix: "<-03>3",
    },
    Timezone {
        iana: "America/Argentina/Cordoba",
        posix: "<-03>3",
    },
    Timezone {
        iana: "America/Argentina/Jujuy",
        posix: "<-03>3",
    },
    Timezone {
        iana: "America/Argentina/La Rioja",
        posix: "<-03>3",
    },
    Timezone {
        iana: "America/Argentina/Mendoza",
        posix: "<-03>3",
    },
    Timezone {
        iana: "America/Argentina/Rio Gallegos",
        posix: "<-03>3",
    },
    Timezone {
        iana: "America/Argentina/Salta",
        posix: "<-03>3",
    },
    Timezone {
        iana: "America/Argentina/San Juan",
        posix: "<-03>3",
    },
    Timezone {
        iana: "America/Argentina/San Luis",
        posix: "<-03>3",
    },
    Timezone {
        iana: "America/Argentina/Tucuman",
        posix: "<-03>3",
    },
    Timezone {
        iana: "America/Argentina/Ushuaia",
        posix: "<-03>3",
    },
    Timezone {
        iana: "America/Aruba",
        posix: "AST4",
    },
    Timezone {
        iana: "America/Asuncion",
        posix: "<-04>4<-03>,M10.1.0/0,M3.4.0/0",
    },
    Timezone {
        iana: "America/Atikokan",
        posix: "EST5",
    },
    Timezone {
        iana: "America/Bahia",
        posix: "<-03>3",
    },
    Timezone {
        iana: "America/Bahia Banderas",
        posix: "CST6CDT,M4.1.0,M10.5.0",
    },
    Timezone {
        iana: "America/Barbados",
        posix: "AST4",
    },
    Timezone {
        iana: "America/Belem",
        posix: "<-03>3",
    },
    Timezone {
        iana: "America/Belize",
        posix: "CST6",
    },
    Timezone {
        iana: "America/Blanc-Sablon",
        posix: "AST4",
    },
    Timezone {
        iana: "America/Boa Vista",
        posix: "<-04>4",
    },
    Timezone {
        iana: "America/Bogota",
        posix: "<-05>5",
    },
    Timezone {
        iana: "America/Boise",
        posix: "MST7MDT,M3.2.0,M11.1.0",
    },
    Timezone {
        iana: "America/Cambridge Bay",
        posix: "MST7MDT,M3.2.0,M11.1.0",
    },
    Timezone {
        iana: "America/Campo Grande",
        posix: "<-04>4",
    },
    Timezone {
        iana: "America/Cancun",
        posix: "EST5",
    },
    Timezone {
        iana: "America/Caracas",
        posix: "<-04>4",
    },
    Timezone {
        iana: "America/Cayenne",
        posix: "<-03>3",
    },
    Timezone {
        iana: "America/Cayman",
        posix: "EST5",
    },
    Timezone {
        iana: "America/Chicago",
        posix: "CST6CDT,M3.2.0,M11.1.0",
    },
    Timezone {
        iana: "America/Chihuahua",
        posix: "MST7MDT,M4.1.0,M10.5.0",
    },
    Timezone {
        iana: "America/Costa Rica",
        posix: "CST6",
    },
    Timezone {
        iana: "America/Creston",
        posix: "MST7",
    },
    Timezone {
        iana: "America/Cuiaba",
        posix: "<-04>4",
    },
    Timezone {
        iana: "America/Curacao",
        posix: "AST4",
    },
    Timezone {
        iana: "America/Danmarkshavn",
        posix: "GMT0",
    },
    Timezone {
        iana: "America/Dawson",
        posix: "MST7",
    },
    Timezone {
        iana: "America/Dawson Creek",
        posix: "MST7",
    },
    Timezone {
        iana: "America/Denver",
        posix: "MST7MDT,M3.2.0,M11.1.0",
    },
    Timezone {
        iana: "America/Detroit",
        posix: "EST5EDT,M3.2.0,M11.1.0",
    },
    Timezone {
        iana: "America/Dominica",
        posix: "AST4",
    },
    Timezone {
        iana: "America/Edmonton",
        posix: "MST7MDT,M3.2.0,M11.1.0",
    },
    Timezone {
        iana: "America/Eirunepe",
        posix: "<-05>5",
    },
    Timezone {
        iana: "America/El Salvador",
        posix: "CST6",
    },
    Timezone {
        iana: "America/Fort Nelson",
        posix: "MST7",
    },
    Timezone {
        iana: "America/Fortaleza",
        posix: "<-03>3",
    },
    Timezone {
        iana: "America/Glace Bay",
        posix: "AST4ADT,M3.2.0,M11.1.0",
    },
    Timezone {
        iana: "America/Goose Bay",
        posix: "AST4ADT,M3.2.0,M11.1.0",
    },
    Timezone {
        iana: "America/Grand Turk",
        posix: "EST5EDT,M3.2.0,M11.1.0",
    },
    Timezone {
        iana: "America/Grenada",
        posix: "AST4",
    },
    Timezone {
        iana: "America/Guadeloupe",
        posix: "AST4",
    },
    Timezone {
        iana: "America/Guatemala",
        posix: "CST6",
    },
    Timezone {
        iana: "America/Guayaquil",
        posix: "<-05>5",
    },
    Timezone {
        iana: "America/Guyana",
        posix: "<-04>4",
    },
    Timezone {
        iana: "America/Halifax",
        posix: "AST4ADT,M3.2.0,M11.1.0",
    },
    Timezone {
        iana: "America/Havana",
        posix: "CST5CDT,M3.2.0/0,M11.1.0/1",
    },
    Timezone {
        iana: "America/Hermosillo",
        posix: "MST7",
    },
    Timezone {
        iana: "America/Indiana/Indianapolis",
        posix: "EST5EDT,M3.2.0,M11.1.0",
    },
    Timezone {
        iana: "America/Indiana/Knox",
        posix: "CST6CDT,M3.2.0,M11.1.0",
    },
    Timezone {
        iana: "America/Indiana/Marengo",
        posix: "EST5EDT,M3.2.0,M11.1.0",
    },
    Timezone {
        iana: "America/Indiana/Petersburg",
        posix: "EST5EDT,M3.2.0,M11.1.0",
    },
    Timezone {
        iana: "America/Indiana/Tell City",
        posix: "CST6CDT,M3.2.0,M11.1.0",
    },
    Timezone {
        iana: "America/Indiana/Vevay",
        posix: "EST5EDT,M3.2.0,M11.1.0",
    },
    Timezone {
        iana: "America/Indiana/Vincennes",
        posix: "EST5EDT,M3.2.0,M11.1.0",
    },
    Timezone {
        iana: "America/Indiana/Winamac",
        posix: "EST5EDT,M3.2.0,M11.1.0",
    },
    Timezone {
        iana: "America/Inuvik",
        posix: "MST7MDT,M3.2.0,M11.1.0",
    },
    Timezone {
        iana: "America/Iqaluit",
        posix: "EST5EDT,M3.2.0,M11.1.0",
    },
    Timezone {
        iana: "America/Jamaica",
        posix: "EST5",
    },
    Timezone {
        iana: "America/Juneau",
        posix: "AKST9AKDT,M3.2.0,M11.1.0",
    },
    Timezone {
        iana: "America/Kentucky/Louisville",
        posix: "EST5EDT,M3.2.0,M11.1.0",
    },
    Timezone {
        iana: "America/Kentucky/Monticello",
        posix: "EST5EDT,M3.2.0,M11.1.0",
    },
    Timezone {
        iana: "America/Kralendijk",
        posix: "AST4",
    },
    Timezone {
        iana: "America/La Paz",
        posix: "<-04>4",
    },
    Timezone {
        iana: "America/Lima",
        posix: "<-05>5",
    },
    Timezone {
        iana: "America/Los Angeles",
        posix: "PST8PDT,M3.2.0,M11.1.0",
    },
    Timezone {
        iana: "America/Lower Princes",
        posix: "AST4",
    },
    Timezone {
        iana: "America/Maceio",
        posix: "<-03>3",
    },
    Timezone {
        iana: "America/Managua",
        posix: "CST6",
    },
    Timezone {
        iana: "America/Manaus",
        posix: "<-04>4",
    },
    Timezone {
        iana: "America/Marigot",
        posix: "AST4",
    },
    Timezone {
        iana: "America/Martinique",
        posix: "AST4",
    },
    Timezone {
        iana: "America/Matamoros",
        posix: "CST6CDT,M3.2.0,M11.1.0",
    },
    Timezone {
        iana: "America/Mazatlan",
        posix: "MST7MDT,M4.1.0,M10.5.0",
    },
    Timezone {
        iana: "America/Menominee",
        posix: "CST6CDT,M3.2.0,M11.1.0",
    },
    Timezone {
        iana: "America/Merida",
        posix: "CST6CDT,M4.1.0,M10.5.0",
    },
    Timezone {
        iana: "America/Metlakatla",
        posix: "AKST9AKDT,M3.2.0,M11.1.0",
    },
    Timezone {
        iana: "America/Mexico City",
        posix: "CST6CDT,M4.1.0,M10.5.0",
    },
    Timezone {
        iana: "America/Miquelon",
        posix: "<-03>3<-02>,M3.2.0,M11.1.0",
    },
    Timezone {
        iana: "America/Moncton",
        posix: "AST4ADT,M3.2.0,M11.1.0",
    },
    Timezone {
        iana: "America/Monterrey",
        posix: "CST6CDT,M4.1.0,M10.5.0",
    },
    Timezone {
        iana: "America/Montevideo",
        posix: "<-03>3",
    },
    Timezone {
        iana: "America/Montserrat",
        posix: "AST4",
    },
    Timezone {
        iana: "America/Nassau",
        posix: "EST5EDT,M3.2.0,M11.1.0",
    },
    Timezone {
        iana: "America/New York",
        posix: "EST5EDT,M3.2.0,M11.1.0",
    },
    Timezone {
        iana: "America/Nipigon",
        posix: "EST5EDT,M3.2.0,M11.1.0",
    },
    Timezone {
        iana: "America/Nome",
        posix: "AKST9AKDT,M3.2.0,M11.1.0",
    },
    Timezone {
        iana: "America/Noronha",
        posix: "<-02>2",
    },
    Timezone {
        iana: "America/North Dakota/Beulah",
        posix: "CST6CDT,M3.2.0,M11.1.0",
    },
    Timezone {
        iana: "America/North Dakota/Center",
        posix: "CST6CDT,M3.2.0,M11.1.0",
    },
    Timezone {
        iana: "America/North Dakota/New Salem",
        posix: "CST6CDT,M3.2.0,M11.1.0",
    },
    Timezone {
        iana: "America/Nuuk",
        posix: "<-03>3<-02>,M3.5.0/-2,M10.5.0/-1",
    },
    Timezone {
        iana: "America/Ojinaga",
        posix: "MST7MDT,M3.2.0,M11.1.0",
    },
    Timezone {
        iana: "America/Panama",
        posix: "EST5",
    },
    Timezone {
        iana: "America/Pangnirtung",
        posix: "EST5EDT,M3.2.0,M11.1.0",
    },
    Timezone {
        iana: "America/Paramaribo",
        posix: "<-03>3",
    },
    Timezone {
        iana: "America/Phoenix",
        posix: "MST7",
    },
    Timezone {
        iana: "America/Port of Spain",
        posix: "AST4",
    },
    Timezone {
        iana: "America/Port-au-Prince",
        posix: "EST5EDT,M3.2.0,M11.1.0",
    },
    Timezone {
        iana: "America/Porto Velho",
        posix: "<-04>4",
    },
    Timezone {
        iana: "America/Puerto Rico",
        posix: "AST4",
    },
    Timezone {
        iana: "America/Punta Arenas",
        posix: "<-03>3",
    },
    Timezone {
        iana: "America/Rainy River",
        posix: "CST6CDT,M3.2.0,M11.1.0",
    },
    Timezone {
        iana: "America/Rankin Inlet",
        posix: "CST6CDT,M3.2.0,M11.1.0",
    },
    Timezone {
        iana: "America/Recife",
        posix: "<-03>3",
    },
    Timezone {
        iana: "America/Regina",
        posix: "CST6",
    },
    Timezone {
        iana: "America/Resolute",
        posix: "CST6CDT,M3.2.0,M11.1.0",
    },
    Timezone {
        iana: "America/Rio Branco",
        posix: "<-05>5",
    },
    Timezone {
        iana: "America/Santarem",
        posix: "<-03>3",
    },
    Timezone {
        iana: "America/Santiago",
        posix: "<-04>4<-03>,M9.1.6/24,M4.1.6/24",
    },
    Timezone {
        iana: "America/Santo Domingo",
        posix: "AST4",
    },
    Timezone {
        iana: "America/Sao Paulo",
        posix: "<-03>3",
    },
    Timezone {
        iana: "America/Scoresbysund",
        posix: "<-01>1<+00>,M3.5.0/0,M10.5.0/1",
    },
    Timezone {
        iana: "America/Sitka",
        posix: "AKST9AKDT,M3.2.0,M11.1.0",
    },
    Timezone {
        iana: "America/St Barthelemy",
        posix: "AST4",
    },
    Timezone {
        iana: "America/St Johns",
        posix: "NST3:30NDT,M3.2.0,M11.1.0",
    },
    Timezone {
        iana: "America/St Kitts",
        posix: "AST4",
    },
    Timezone {
        iana: "America/St Lucia",
        posix: "AST4",
    },
    Timezone {
        iana: "America/St Thomas",
        posix: "AST4",
    },
    Timezone {
        iana: "America/St Vincent",
        posix: "AST4",
    },
    Timezone {
        iana: "America/Swift Current",
        posix: "CST6",
    },
    Timezone {
        iana: "America/Tegucigalpa",
        posix: "CST6",
    },
    Timezone {
        iana: "America/Thule",
        posix: "AST4ADT,M3.2.0,M11.1.0",
    },
    Timezone {
        iana: "America/Thunder Bay",
        posix: "EST5EDT,M3.2.0,M11.1.0",
    },
    Timezone {
        iana: "America/Tijuana",
        posix: "PST8PDT,M3.2.0,M11.1.0",
    },
    Timezone {
        iana: "America/Toronto",
        posix: "EST5EDT,M3.2.0,M11.1.0",
    },
    Timezone {
        iana: "America/Tortola",
        posix: "AST4",
    },
    Timezone {
        iana: "America/Vancouver",
        posix: "PST8PDT,M3.2.0,M11.1.0",
    },
    Timezone {
        iana: "America/Whitehorse",
        posix: "MST7",
    },
    Timezone {
        iana: "America/Winnipeg",
        posix: "CST6CDT,M3.2.0,M11.1.0",
    },
    Timezone {
        iana: "America/Yakutat",
        posix: "AKST9AKDT,M3.2.0,M11.1.0",
    },
    Timezone {
        iana: "America/Yellowknife",
        posix: "MST7MDT,M3.2.0,M11.1.0",
    },
    Timezone {
        iana: "Antarctica/Casey",
        posix: "<+11>-11",
    },
    Timezone {
        iana: "Antarctica/Davis",
        posix: "<+07>-7",
    },
    Timezone {
        iana: "Antarctica/DumontDUrville",
        posix: "<+10>-10",
    },
    Timezone {
        iana: "Antarctica/Macquarie",
        posix: "AEST-10AEDT,M10.1.0,M4.1.0/3",
    },
    Timezone {
        iana: "Antarctica/Mawson",
        posix: "<+05>-5",
    },
    Timezone {
        iana: "Antarctica/McMurdo",
        posix: "NZST-12NZDT,M9.5.0,M4.1.0/3",
    },
    Timezone {
        iana: "Antarctica/Palmer",
        posix: "<-03>3",
    },
    Timezone {
        iana: "Antarctica/Rothera",
        posix: "<-03>3",
    },
    Timezone {
        iana: "Antarctica/Syowa",
        posix: "<+03>-3",
    },
    Timezone {
        iana: "Antarctica/Troll",
        posix: "<+00>0<+02>-2,M3.5.0/1,M10.5.0/3",
    },
    Timezone {
        iana: "Antarctica/Vostok",
        posix: "<+06>-6",
    },
    Timezone {
        iana: "Arctic/Longyearbyen",
        posix: "CET-1CEST,M3.5.0,M10.5.0/3",
    },
    Timezone {
        iana: "Asia/Aden",
        posix: "<+03>-3",
    },
    Timezone {
        iana: "Asia/Almaty",
        posix: "<+06>-6",
    },
    Timezone {
        iana: "Asia/Amman",
        posix: "<+03>-3",
    },
    Timezone {
        iana: "Asia/Anadyr",
        posix: "<+12>-12",
    },
    Timezone {
        iana: "Asia/Aqtau",
        posix: "<+05>-5",
    },
    Timezone {
        iana: "Asia/Aqtobe",
        posix: "<+05>-5",
    },
    Timezone {
        iana: "Asia/Ashgabat",
        posix: "<+05>-5",
    },
    Timezone {
        iana: "Asia/Atyrau",
        posix: "<+05>-5",
    },
    Timezone {
        iana: "Asia/Baghdad",
        posix: "<+03>-3",
    },
    Timezone {
        iana: "Asia/Bahrain",
        posix: "<+03>-3",
    },
    Timezone {
        iana: "Asia/Baku",
        posix: "<+04>-4",
    },
    Timezone {
        iana: "Asia/Bangkok",
        posix: "<+07>-7",
    },
    Timezone {
        iana: "Asia/Barnaul",
        posix: "<+07>-7",
    },
    Timezone {
        iana: "Asia/Beirut",
        posix: "EET-2EEST,M3.5.0/0,M10.5.0/0",
    },
    Timezone {
        iana: "Asia/Bishkek",
        posix: "<+06>-6",
    },
    Timezone {
        iana: "Asia/Brunei",
        posix: "<+08>-8",
    },
    Timezone {
        iana: "Asia/Chita",
        posix: "<+09>-9",
    },
    Timezone {
        iana: "Asia/Choibalsan",
        posix: "<+08>-8",
    },
    Timezone {
        iana: "Asia/Colombo",
        posix: "<+0530>-5:30",
    },
    Timezone {
        iana: "Asia/Damascus",
        posix: "<+03>-3",
    },
    Timezone {
        iana: "Asia/Dhaka",
        posix: "<+06>-6",
    },
    Timezone {
        iana: "Asia/Dili",
        posix: "<+09>-9",
    },
    Timezone {
        iana: "Asia/Dubai",
        posix: "<+04>-4",
    },
    Timezone {
        iana: "Asia/Dushanbe",
        posix: "<+05>-5",
    },
    Timezone {
        iana: "Asia/Famagusta",
        posix: "EET-2EEST,M3.5.0/3,M10.5.0/4",
    },
    Timezone {
        iana: "Asia/Gaza",
        posix: "EET-2EEST,M3.4.4/50,M10.4.4/50",
    },
    Timezone {
        iana: "Asia/Hebron",
        posix: "EET-2EEST,M3.4.4/50,M10.4.4/50",
    },
    Timezone {
        iana: "Asia/Ho Chi Minh",
        posix: "<+07>-7",
    },
    Timezone {
        iana: "Asia/Hong Kong",
        posix: "HKT-8",
    },
    Timezone {
        iana: "Asia/Hovd",
        posix: "<+07>-7",
    },
    Timezone {
        iana: "Asia/Irkutsk",
        posix: "<+08>-8",
    },
    Timezone {
        iana: "Asia/Jakarta",
        posix: "WIB-7",
    },
    Timezone {
        iana: "Asia/Jayapura",
        posix: "WIT-9",
    },
    Timezone {
        iana: "Asia/Jerusalem",
        posix: "IST-2IDT,M3.4.4/26,M10.5.0",
    },
    Timezone {
        iana: "Asia/Kabul",
        posix: "<+0430>-4:30",
    },
    Timezone {
        iana: "Asia/Kamchatka",
        posix: "<+12>-12",
    },
    Timezone {
        iana: "Asia/Karachi",
        posix: "PKT-5",
    },
    Timezone {
        iana: "Asia/Kathmandu",
        posix: "<+0545>-5:45",
    },
    Timezone {
        iana: "Asia/Khandyga",
        posix: "<+09>-9",
    },
    Timezone {
        iana: "Asia/Kolkata",
        posix: "IST-5:30",
    },
    Timezone {
        iana: "Asia/Krasnoyarsk",
        posix: "<+07>-7",
    },
    Timezone {
        iana: "Asia/Kuala Lumpur",
        posix: "<+08>-8",
    },
    Timezone {
        iana: "Asia/Kuching",
        posix: "<+08>-8",
    },
    Timezone {
        iana: "Asia/Kuwait",
        posix: "<+03>-3",
    },
    Timezone {
        iana: "Asia/Macau",
        posix: "CST-8",
    },
    Timezone {
        iana: "Asia/Magadan",
        posix: "<+11>-11",
    },
    Timezone {
        iana: "Asia/Makassar",
        posix: "WITA-8",
    },
    Timezone {
        iana: "Asia/Manila",
        posix: "PST-8",
    },
    Timezone {
        iana: "Asia/Muscat",
        posix: "<+04>-4",
    },
    Timezone {
        iana: "Asia/Nicosia",
        posix: "EET-2EEST,M3.5.0/3,M10.5.0/4",
    },
    Timezone {
        iana: "Asia/Novokuznetsk",
        posix: "<+07>-7",
    },
    Timezone {
        iana: "Asia/Novosibirsk",
        posix: "<+07>-7",
    },
    Timezone {
        iana: "Asia/Omsk",
        posix: "<+06>-6",
    },
    Timezone {
        iana: "Asia/Oral",
        posix: "<+05>-5",
    },
    Timezone {
        iana: "Asia/Phnom Penh",
        posix: "<+07>-7",
    },
    Timezone {
        iana: "Asia/Pontianak",
        posix: "WIB-7",
    },
    Timezone {
        iana: "Asia/Pyongyang",
        posix: "KST-9",
    },
    Timezone {
        iana: "Asia/Qatar",
        posix: "<+03>-3",
    },
    Timezone {
        iana: "Asia/Qostanay",
        posix: "<+06>-6",
    },
    Timezone {
        iana: "Asia/Qyzylorda",
        posix: "<+05>-5",
    },
    Timezone {
        iana: "Asia/Riyadh",
        posix: "<+03>-3",
    },
    Timezone {
        iana: "Asia/Sakhalin",
        posix: "<+11>-11",
    },
    Timezone {
        iana: "Asia/Samarkand",
        posix: "<+05>-5",
    },
    Timezone {
        iana: "Asia/Seoul",
        posix: "KST-9",
    },
    Timezone {
        iana: "Asia/Shanghai",
        posix: "CST-8",
    },
    Timezone {
        iana: "Asia/Singapore",
        posix: "<+08>-8",
    },
    Timezone {
        iana: "Asia/Srednekolymsk",
        posix: "<+11>-11",
    },
    Timezone {
        iana: "Asia/Taipei",
        posix: "CST-8",
    },
    Timezone {
        iana: "Asia/Tashkent",
        posix: "<+05>-5",
    },
    Timezone {
        iana: "Asia/Tbilisi",
        posix: "<+04>-4",
    },
    Timezone {
        iana: "Asia/Tehran",
        posix: "<+0330>-3:30",
    },
    Timezone {
        iana: "Asia/Thimphu",
        posix: "<+06>-6",
    },
    Timezone {
        iana: "Asia/Tokyo",
        posix: "JST-9",
    },
    Timezone {
        iana: "Asia/Tomsk",
        posix: "<+07>-7",
    },
    Timezone {
        iana: "Asia/Ulaanbaatar",
        posix: "<+08>-8",
    },
    Timezone {
        iana: "Asia/Urumqi",
        posix: "<+06>-6",
    },
    Timezone {
        iana: "Asia/Ust-Nera",
        posix: "<+10>-10",
    },
    Timezone {
        iana: "Asia/Vientiane",
        posix: "<+07>-7",
    },
    Timezone {
        iana: "Asia/Vladivostok",
        posix: "<+10>-10",
    },
    Timezone {
        iana: "Asia/Yakutsk",
        posix: "<+09>-9",
    },
    Timezone {
        iana: "Asia/Yangon",
        posix: "<+0630>-6:30",
    },
    Timezone {
        iana: "Asia/Yekaterinburg",
        posix: "<+05>-5",
    },
    Timezone {
        iana: "Asia/Yerevan",
        posix: "<+04>-4",
    },
    Timezone {
        iana: "Atlantic/Azores",
        posix: "<-01>1<+00>,M3.5.0/0,M10.5.0/1",
    },
    Timezone {
        iana: "Atlantic/Bermuda",
        posix: "AST4ADT,M3.2.0,M11.1.0",
    },
    Timezone {
        iana: "Atlantic/Canary",
        posix: "WET0WEST,M3.5.0/1,M10.5.0",
    },
    Timezone {
        iana: "Atlantic/Cape Verde",
        posix: "<-01>1",
    },
    Timezone {
        iana: "Atlantic/Faroe",
        posix: "WET0WEST,M3.5.0/1,M10.5.0",
    },
    Timezone {
        iana: "Atlantic/Madeira",
        posix: "WET0WEST,M3.5.0/1,M10.5.0",
    },
    Timezone {
        iana: "Atlantic/Reykjavik",
        posix: "GMT0",
    },
    Timezone {
        iana: "Atlantic/South Georgia",
        posix: "<-02>2",
    },
    Timezone {
        iana: "Atlantic/St Helena",
        posix: "GMT0",
    },
    Timezone {
        iana: "Atlantic/Stanley",
        posix: "<-03>3",
    },
    Timezone {
        iana: "Australia/Adelaide",
        posix: "ACST-9:30ACDT,M10.1.0,M4.1.0/3",
    },
    Timezone {
        iana: "Australia/Brisbane",
        posix: "AEST-10",
    },
    Timezone {
        iana: "Australia/Broken Hill",
        posix: "ACST-9:30ACDT,M10.1.0,M4.1.0/3",
    },
    Timezone {
        iana: "Australia/Darwin",
        posix: "ACST-9:30",
    },
    Timezone {
        iana: "Australia/Eucla",
        posix: "<+0845>-8:45",
    },
    Timezone {
        iana: "Australia/Hobart",
        posix: "AEST-10AEDT,M10.1.0,M4.1.0/3",
    },
    Timezone {
        iana: "Australia/Lindeman",
        posix: "AEST-10",
    },
    Timezone {
        iana: "Australia/Lord Howe",
        posix: "<+1030>-10:30<+11>-11,M10.1.0,M4.1.0",
    },
    Timezone {
        iana: "Australia/Melbourne",
        posix: "AEST-10AEDT,M10.1.0,M4.1.0/3",
    },
    Timezone {
        iana: "Australia/Perth",
        posix: "AWST-8",
    },
    Timezone {
        iana: "Australia/Sydney",
        posix: "AEST-10AEDT,M10.1.0,M4.1.0/3",
    },
    Timezone {
        iana: "Etc/GMT",
        posix: "GMT0",
    },
    Timezone {
        iana: "Etc/GMT+1",
        posix: "<-01>1",
    },
    Timezone {
        iana: "Etc/GMT+10",
        posix: "<-10>10",
    },
    Timezone {
        iana: "Etc/GMT+11",
        posix: "<-11>11",
    },
    Timezone {
        iana: "Etc/GMT+12",
        posix: "<-12>12",
    },
    Timezone {
        iana: "Etc/GMT+2",
        posix: "<-02>2",
    },
    Timezone {
        iana: "Etc/GMT+3",
        posix: "<-03>3",
    },
    Timezone {
        iana: "Etc/GMT+4",
        posix: "<-04>4",
    },
    Timezone {
        iana: "Etc/GMT+5",
        posix: "<-05>5",
    },
    Timezone {
        iana: "Etc/GMT+6",
        posix: "<-06>6",
    },
    Timezone {
        iana: "Etc/GMT+7",
        posix: "<-07>7",
    },
    Timezone {
        iana: "Etc/GMT+8",
        posix: "<-08>8",
    },
    Timezone {
        iana: "Etc/GMT+9",
        posix: "<-09>9",
    },
    Timezone {
        iana: "Etc/GMT-1",
        posix: "<+01>-1",
    },
    Timezone {
        iana: "Etc/GMT-10",
        posix: "<+10>-10",
    },
    Timezone {
        iana: "Etc/GMT-11",
        posix: "<+11>-11",
    },
    Timezone {
        iana: "Etc/GMT-12",
        posix: "<+12>-12",
    },
    Timezone {
        iana: "Etc/GMT-13",
        posix: "<+13>-13",
    },
    Timezone {
        iana: "Etc/GMT-14",
        posix: "<+14>-14",
    },
    Timezone {
        iana: "Etc/GMT-2",
        posix: "<+02>-2",
    },
    Timezone {
        iana: "Etc/GMT-3",
        posix: "<+03>-3",
    },
    Timezone {
        iana: "Etc/GMT-4",
        posix: "<+04>-4",
    },
    Timezone {
        iana: "Etc/GMT-5",
        posix: "<+05>-5",
    },
    Timezone {
        iana: "Etc/GMT-6",
        posix: "<+06>-6",
    },
    Timezone {
        iana: "Etc/GMT-7",
        posix: "<+07>-7",
    },
    Timezone {
        iana: "Etc/GMT-8",
        posix: "<+08>-8",
    },
    Timezone {
        iana: "Etc/GMT-9",
        posix: "<+09>-9",
    },
    Timezone {
        iana: "Europe/Amsterdam",
        posix: "CET-1CEST,M3.5.0,M10.5.0/3",
    },
    Timezone {
        iana: "Europe/Andorra",
        posix: "CET-1CEST,M3.5.0,M10.5.0/3",
    },
    Timezone {
        iana: "Europe/Astrakhan",
        posix: "<+04>-4",
    },
    Timezone {
        iana: "Europe/Athens",
        posix: "EET-2EEST,M3.5.0/3,M10.5.0/4",
    },
    Timezone {
        iana: "Europe/Belgrade",
        posix: "CET-1CEST,M3.5.0,M10.5.0/3",
    },
    Timezone {
        iana: "Europe/Berlin",
        posix: "CET-1CEST,M3.5.0,M10.5.0/3",
    },
    Timezone {
        iana: "Europe/Bratislava",
        posix: "CET-1CEST,M3.5.0,M10.5.0/3",
    },
    Timezone {
        iana: "Europe/Brussels",
        posix: "CET-1CEST,M3.5.0,M10.5.0/3",
    },
    Timezone {
        iana: "Europe/Bucharest",
        posix: "EET-2EEST,M3.5.0/3,M10.5.0/4",
    },
    Timezone {
        iana: "Europe/Budapest",
        posix: "CET-1CEST,M3.5.0,M10.5.0/3",
    },
    Timezone {
        iana: "Europe/Busingen",
        posix: "CET-1CEST,M3.5.0,M10.5.0/3",
    },
    Timezone {
        iana: "Europe/Chisinau",
        posix: "EET-2EEST,M3.5.0,M10.5.0/3",
    },
    Timezone {
        iana: "Europe/Copenhagen",
        posix: "CET-1CEST,M3.5.0,M10.5.0/3",
    },
    Timezone {
        iana: "Europe/Dublin",
        posix: "IST-1GMT0,M10.5.0,M3.5.0/1",
    },
    Timezone {
        iana: "Europe/Gibraltar",
        posix: "CET-1CEST,M3.5.0,M10.5.0/3",
    },
    Timezone {
        iana: "Europe/Guernsey",
        posix: "GMT0BST,M3.5.0/1,M10.5.0",
    },
    Timezone {
        iana: "Europe/Helsinki",
        posix: "EET-2EEST,M3.5.0/3,M10.5.0/4",
    },
    Timezone {
        iana: "Europe/Isle of Man",
        posix: "GMT0BST,M3.5.0/1,M10.5.0",
    },
    Timezone {
        iana: "Europe/Istanbul",
        posix: "<+03>-3",
    },
    Timezone {
        iana: "Europe/Jersey",
        posix: "GMT0BST,M3.5.0/1,M10.5.0",
    },
    Timezone {
        iana: "Europe/Kaliningrad",
        posix: "EET-2",
    },
    Timezone {
        iana: "Europe/Kirov",
        posix: "<+03>-3",
    },
    Timezone {
        iana: "Europe/Kyiv",
        posix: "EET-2EEST,M3.5.0/3,M10.5.0/4",
    },
    Timezone {
        iana: "Europe/Lisbon",
        posix: "WET0WEST,M3.5.0/1,M10.5.0",
    },
    Timezone {
        iana: "Europe/Ljubljana",
        posix: "CET-1CEST,M3.5.0,M10.5.0/3",
    },
    Timezone {
        iana: "Europe/London",
        posix: "GMT0BST,M3.5.0/1,M10.5.0",
    },
    Timezone {
        iana: "Europe/Luxembourg",
        posix: "CET-1CEST,M3.5.0,M10.5.0/3",
    },
    Timezone {
        iana: "Europe/Madrid",
        posix: "CET-1CEST,M3.5.0,M10.5.0/3",
    },
    Timezone {
        iana: "Europe/Malta",
        posix: "CET-1CEST,M3.5.0,M10.5.0/3",
    },
    Timezone {
        iana: "Europe/Mariehamn",
        posix: "EET-2EEST,M3.5.0/3,M10.5.0/4",
    },
    Timezone {
        iana: "Europe/Minsk",
        posix: "<+03>-3",
    },
    Timezone {
        iana: "Europe/Monaco",
        posix: "CET-1CEST,M3.5.0,M10.5.0/3",
    },
    Timezone {
        iana: "Europe/Moscow",
        posix: "MSK-3",
    },
    Timezone {
        iana: "Europe/Oslo",
        posix: "CET-1CEST,M3.5.0,M10.5.0/3",
    },
    Timezone {
        iana: "Europe/Paris",
        posix: "CET-1CEST,M3.5.0,M10.5.0/3",
    },
    Timezone {
        iana: "Europe/Podgorica",
        posix: "CET-1CEST,M3.5.0,M10.5.0/3",
    },
    Timezone {
        iana: "Europe/Prague",
        posix: "CET-1CEST,M3.5.0,M10.5.0/3",
    },
    Timezone {
        iana: "Europe/Riga",
        posix: "EET-2EEST,M3.5.0/3,M10.5.0/4",
    },
    Timezone {
        iana: "Europe/Rome",
        posix: "CET-1CEST,M3.5.0,M10.5.0/3",
    },
    Timezone {
        iana: "Europe/Samara",
        posix: "<+04>-4",
    },
    Timezone {
        iana: "Europe/San Marino",
        posix: "CET-1CEST,M3.5.0,M10.5.0/3",
    },
    Timezone {
        iana: "Europe/Sarajevo",
        posix: "CET-1CEST,M3.5.0,M10.5.0/3",
    },
    Timezone {
        iana: "Europe/Saratov",
        posix: "<+04>-4",
    },
    Timezone {
        iana: "Europe/Simferopol",
        posix: "MSK-3",
    },
    Timezone {
        iana: "Europe/Skopje",
        posix: "CET-1CEST,M3.5.0,M10.5.0/3",
    },
    Timezone {
        iana: "Europe/Sofia",
        posix: "EET-2EEST,M3.5.0/3,M10.5.0/4",
    },
    Timezone {
        iana: "Europe/Stockholm",
        posix: "CET-1CEST,M3.5.0,M10.5.0/3",
    },
    Timezone {
        iana: "Europe/Tallinn",
        posix: "EET-2EEST,M3.5.0/3,M10.5.0/4",
    },
    Timezone {
        iana: "Europe/Tirane",
        posix: "CET-1CEST,M3.5.0,M10.5.0/3",
    },
    Timezone {
        iana: "Europe/Ulyanovsk",
        posix: "<+04>-4",
    },
    Timezone {
        iana: "Europe/Vaduz",
        posix: "CET-1CEST,M3.5.0,M10.5.0/3",
    },
    Timezone {
        iana: "Europe/Vatican",
        posix: "CET-1CEST,M3.5.0,M10.5.0/3",
    },
    Timezone {
        iana: "Europe/Vienna",
        posix: "CET-1CEST,M3.5.0,M10.5.0/3",
    },
    Timezone {
        iana: "Europe/Vilnius",
        posix: "EET-2EEST,M3.5.0/3,M10.5.0/4",
    },
    Timezone {
        iana: "Europe/Volgograd",
        posix: "<+03>-3",
    },
    Timezone {
        iana: "Europe/Warsaw",
        posix: "CET-1CEST,M3.5.0,M10.5.0/3",
    },
    Timezone {
        iana: "Europe/Zagreb",
        posix: "CET-1CEST,M3.5.0,M10.5.0/3",
    },
    Timezone {
        iana: "Europe/Zurich",
        posix: "CET-1CEST,M3.5.0,M10.5.0/3",
    },
    Timezone {
        iana: "Indian/Antananarivo",
        posix: "EAT-3",
    },
    Timezone {
        iana: "Indian/Chagos",
        posix: "<+06>-6",
    },
    Timezone {
        iana: "Indian/Christmas",
        posix: "<+07>-7",
    },
    Timezone {
        iana: "Indian/Cocos",
        posix: "<+0630>-6:30",
    },
    Timezone {
        iana: "Indian/Comoro",
        posix: "EAT-3",
    },
    Timezone {
        iana: "Indian/Kerguelen",
        posix: "<+05>-5",
    },
    Timezone {
        iana: "Indian/Mahe",
        posix: "<+04>-4",
    },
    Timezone {
        iana: "Indian/Maldives",
        posix: "<+05>-5",
    },
    Timezone {
        iana: "Indian/Mauritius",
        posix: "<+04>-4",
    },
    Timezone {
        iana: "Indian/Mayotte",
        posix: "EAT-3",
    },
    Timezone {
        iana: "Indian/Reunion",
        posix: "<+04>-4",
    },
    Timezone {
        iana: "Pacific/Apia",
        posix: "<+13>-13",
    },
    Timezone {
        iana: "Pacific/Auckland",
        posix: "NZST-12NZDT,M9.5.0,M4.1.0/3",
    },
    Timezone {
        iana: "Pacific/Bougainville",
        posix: "<+11>-11",
    },
    Timezone {
        iana: "Pacific/Chatham",
        posix: "<+1245>-12:45<+1345>,M9.5.0/2:45,M4.1.0/3:45",
    },
    Timezone {
        iana: "Pacific/Chuuk",
        posix: "<+10>-10",
    },
    Timezone {
        iana: "Pacific/Easter",
        posix: "<-06>6<-05>,M9.1.6/22,M4.1.6/22",
    },
    Timezone {
        iana: "Pacific/Efate",
        posix: "<+11>-11",
    },
    Timezone {
        iana: "Pacific/Fakaofo",
        posix: "<+13>-13",
    },
    Timezone {
        iana: "Pacific/Fiji",
        posix: "<+12>-12<+13>,M11.2.0,M1.2.3/99",
    },
    Timezone {
        iana: "Pacific/Funafuti",
        posix: "<+12>-12",
    },
    Timezone {
        iana: "Pacific/Galapagos",
        posix: "<-06>6",
    },
    Timezone {
        iana: "Pacific/Gambier",
        posix: "<-09>9",
    },
    Timezone {
        iana: "Pacific/Guadalcanal",
        posix: "<+11>-11",
    },
    Timezone {
        iana: "Pacific/Guam",
        posix: "ChST-10",
    },
    Timezone {
        iana: "Pacific/Honolulu",
        posix: "HST10",
    },
    Timezone {
        iana: "Pacific/Kanton",
        posix: "<+13>-13",
    },
    Timezone {
        iana: "Pacific/Kiritimati",
        posix: "<+14>-14",
    },
    Timezone {
        iana: "Pacific/Kosrae",
        posix: "<+11>-11",
    },
    Timezone {
        iana: "Pacific/Kwajalein",
        posix: "<+12>-12",
    },
    Timezone {
        iana: "Pacific/Majuro",
        posix: "<+12>-12",
    },
    Timezone {
        iana: "Pacific/Marquesas",
        posix: "<-0930>9:30",
    },
    Timezone {
        iana: "Pacific/Midway",
        posix: "SST11",
    },
    Timezone {
        iana: "Pacific/Nauru",
        posix: "<+12>-12",
    },
    Timezone {
        iana: "Pacific/Niue",
        posix: "<-11>11",
    },
    Timezone {
        iana: "Pacific/Norfolk",
        posix: "<+11>-11<+12>,M10.1.0,M4.1.0/3",
    },
    Timezone {
        iana: "Pacific/Noumea",
        posix: "<+11>-11",
    },
    Timezone {
        iana: "Pacific/Pago Pago",
        posix: "SST11",
    },
    Timezone {
        iana: "Pacific/Palau",
        posix: "<+09>-9",
    },
    Timezone {
        iana: "Pacific/Pitcairn",
        posix: "<-08>8",
    },
    Timezone {
        iana: "Pacific/Pohnpei",
        posix: "<+11>-11",
    },
    Timezone {
        iana: "Pacific/Port Moresby",
        posix: "<+10>-10",
    },
    Timezone {
        iana: "Pacific/Rarotonga",
        posix: "<-10>10",
    },
    Timezone {
        iana: "Pacific/Saipan",
        posix: "ChST-10",
    },
    Timezone {
        iana: "Pacific/Tahiti",
        posix: "<-10>10",
    },
    Timezone {
        iana: "Pacific/Tarawa",
        posix: "<+12>-12",
    },
    Timezone {
        iana: "Pacific/Tongatapu",
        posix: "<+13>-13",
    },
    Timezone {
        iana: "Pacific/Wake",
        posix: "<+12>-12",
    },
    Timezone {
        iana: "Pacific/Wallis",
        posix: "<+12>-12",
    },
];
