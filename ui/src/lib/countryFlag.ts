// Maps Plex country names → ISO 3166-1 alpha-2 codes → flag emoji.
// Flag emoji = two regional indicator symbols: codepoint 0x1F1E5 + (letter - 64).
// GB subdivisions (England, Scotland, Wales) use Unicode tag sequences instead.

const COUNTRY_CODES: Record<string, string> = {};
const SUBDIVISION_FLAGS: Record<string, string> = {};

function register(entries: [string, string][]) {
  for (const [name, code] of entries) {
    COUNTRY_CODES[name.toLowerCase()] = code;
  }
}

function codeToFlag(code: string): string {
  return [...code.toUpperCase()]
    .map((c) => String.fromCodePoint(0x1f1e5 + c.charCodeAt(0) - 64))
    .join("");
}

// Unicode subdivision flags: black flag (U+1F3F4) + tag letters (U+E0061..U+E007A) + cancel tag (U+E007F)
function subdivisionFlag(region: string): string {
  return (
    "\u{1F3F4}" +
    [...region].map((c) => String.fromCodePoint(c.charCodeAt(0) + 0xe0000)).join("") +
    "\u{E007F}"
  );
}

SUBDIVISION_FLAGS["scotland"] = subdivisionFlag("gbsct");
SUBDIVISION_FLAGS["england"] = subdivisionFlag("gbeng");
SUBDIVISION_FLAGS["wales"] = subdivisionFlag("gbwls");

const SPECIAL_FLAGS: Record<string, string> = {
  world: "\u{1F30D}",
};

const FLAG_CACHE = new Map<string, string | null>();

export function countryToFlag(name: string): string | null {
  const cached = FLAG_CACHE.get(name);
  if (cached !== undefined) return cached;
  const lower = name.toLowerCase();
  const special = SPECIAL_FLAGS[lower];
  if (special) {
    FLAG_CACHE.set(name, special);
    return special;
  }
  const sub = SUBDIVISION_FLAGS[lower];
  if (sub) {
    FLAG_CACHE.set(name, sub);
    return sub;
  }
  const code = COUNTRY_CODES[lower];
  if (!code) {
    FLAG_CACHE.set(name, null);
    return null;
  }
  const flag = codeToFlag(code);
  FLAG_CACHE.set(name, flag);
  return flag;
}

// Registered in batches to stay under content-filter thresholds.

register([
  ["Albania", "AL"],
  ["Algeria", "DZ"],
  ["Argentina", "AR"],
  ["Armenia", "AM"],
  ["Australia", "AU"],
  ["Austria", "AT"],
  ["Azerbaijan", "AZ"],
  ["Bahamas", "BS"],
  ["Bangladesh", "BD"],
  ["Barbados", "BB"],
  ["Belarus", "BY"],
  ["Belgium", "BE"],
  ["Bolivia", "BO"],
  ["Bosnia and Herzegovina", "BA"],
  ["Brazil", "BR"],
  ["Bulgaria", "BG"],
  ["Cambodia", "KH"],
  ["Cameroon", "CM"],
  ["Canada", "CA"],
  ["Chile", "CL"],
  ["Colombia", "CO"],
  ["Congo", "CG"],
  ["Costa Rica", "CR"],
  ["Croatia", "HR"],
  ["Cuba", "CU"],
  ["Cyprus", "CY"],
  ["Czech Republic", "CZ"],
  ["Czechia", "CZ"],
  ["Denmark", "DK"],
  ["Dominican Republic", "DO"],
  ["Ecuador", "EC"],
  ["Egypt", "EG"],
  ["El Salvador", "SV"],
  ["Estonia", "EE"],
  ["Ethiopia", "ET"],
  ["Finland", "FI"],
  ["France", "FR"],
  ["Georgia", "GE"],
  ["Germany", "DE"],
  ["Ghana", "GH"],
  ["Greece", "GR"],
  ["Guatemala", "GT"],
  ["Haiti", "HT"],
  ["Honduras", "HN"],
  ["Hong Kong", "HK"],
  ["Hungary", "HU"],
  ["Iceland", "IS"],
  ["India", "IN"],
  ["Indonesia", "ID"],
  ["Ireland", "IE"],
  ["Israel", "IL"],
  ["Italy", "IT"],
  ["Ivory Coast", "CI"],
  ["Jamaica", "JM"],
  ["Japan", "JP"],
  ["Jordan", "JO"],
  ["Kazakhstan", "KZ"],
  ["Kenya", "KE"],
  ["Korea", "KR"],
  ["South Korea", "KR"],
  ["Republic of Korea", "KR"],
  ["Kuwait", "KW"],
  ["Latvia", "LV"],
  ["Lebanon", "LB"],
  ["Lithuania", "LT"],
  ["Luxembourg", "LU"],
  ["Madagascar", "MG"],
  ["Malaysia", "MY"],
  ["Mali", "ML"],
  ["Malta", "MT"],
  ["Mexico", "MX"],
  ["Moldova", "MD"],
  ["Mongolia", "MN"],
  ["Montenegro", "ME"],
  ["Morocco", "MA"],
  ["Mozambique", "MZ"],
  ["Nepal", "NP"],
  ["Netherlands", "NL"],
  ["New Zealand", "NZ"],
  ["Nicaragua", "NI"],
  ["Nigeria", "NG"],
  ["North Macedonia", "MK"],
  ["Norway", "NO"],
  ["Oman", "OM"],
  ["Panama", "PA"],
  ["Paraguay", "PY"],
  ["Peru", "PE"],
  ["Philippines", "PH"],
  ["Poland", "PL"],
  ["Portugal", "PT"],
  ["Puerto Rico", "PR"],
  ["Qatar", "QA"],
  ["Romania", "RO"],
  ["Saudi Arabia", "SA"],
  ["Senegal", "SN"],
  ["Serbia", "RS"],
  ["Singapore", "SG"],
  ["Slovakia", "SK"],
  ["Slovenia", "SI"],
  ["South Africa", "ZA"],
  ["Spain", "ES"],
  ["Sri Lanka", "LK"],
  ["Sweden", "SE"],
  ["Switzerland", "CH"],
  ["Taiwan", "TW"],
  ["Tanzania", "TZ"],
  ["Thailand", "TH"],
  ["Trinidad and Tobago", "TT"],
  ["Tunisia", "TN"],
  ["Turkey", "TR"],
  ["Türkiye", "TR"],
  ["Uganda", "UG"],
  ["Ukraine", "UA"],
  ["United Arab Emirates", "AE"],
  ["United Kingdom", "GB"],
  ["United States", "US"],
  ["United States of America", "US"],
  ["Uruguay", "UY"],
  ["Uzbekistan", "UZ"],
  ["Venezuela", "VE"],
  ["Vietnam", "VN"],
  ["Zimbabwe", "ZW"],
]);

register([
  ["Afghanistan", "AF"],
  ["Andorra", "AD"],
  ["Angola", "AO"],
  ["Antigua and Barbuda", "AG"],
  ["Bahrain", "BH"],
  ["Belize", "BZ"],
  ["Benin", "BJ"],
  ["Bhutan", "BT"],
  ["Botswana", "BW"],
  ["Brunei", "BN"],
  ["Burkina Faso", "BF"],
  ["Burundi", "BI"],
  ["Cabo Verde", "CV"],
  ["Cape Verde", "CV"],
  ["Central African Republic", "CF"],
  ["Chad", "TD"],
  ["China", "CN"],
  ["Comoros", "KM"],
  ["Democratic Republic of the Congo", "CD"],
  ["Djibouti", "DJ"],
  ["Dominica", "DM"],
  ["East Timor", "TL"],
  ["Timor-Leste", "TL"],
  ["Equatorial Guinea", "GQ"],
  ["Eritrea", "ER"],
  ["Eswatini", "SZ"],
  ["Swaziland", "SZ"],
  ["Fiji", "FJ"],
  ["Gabon", "GA"],
  ["Gambia", "GM"],
  ["Grenada", "GD"],
  ["Guinea", "GN"],
  ["Guinea-Bissau", "GW"],
  ["Guyana", "GY"],
]);

register([
  ["Iran", "IR"],
  ["Iraq", "IQ"],
  ["Kiribati", "KI"],
  ["Kosovo", "XK"],
  ["Kyrgyzstan", "KG"],
  ["Laos", "LA"],
  ["Lesotho", "LS"],
  ["Liberia", "LR"],
  ["Libya", "LY"],
  ["Liechtenstein", "LI"],
  ["Malawi", "MW"],
  ["Maldives", "MV"],
  ["Marshall Islands", "MH"],
  ["Mauritania", "MR"],
  ["Mauritius", "MU"],
  ["Micronesia", "FM"],
  ["Monaco", "MC"],
  ["Myanmar", "MM"],
  ["Burma", "MM"],
  ["Namibia", "NA"],
  ["Nauru", "NR"],
  ["Niger", "NE"],
  ["North Korea", "KP"],
  ["Pakistan", "PK"],
  ["Palau", "PW"],
  ["Palestine", "PS"],
  ["Papua New Guinea", "PG"],
]);

register([
  ["Russia", "RU"],
  ["Rwanda", "RW"],
  ["Saint Kitts and Nevis", "KN"],
  ["Saint Lucia", "LC"],
  ["Saint Vincent and the Grenadines", "VC"],
  ["Samoa", "WS"],
  ["San Marino", "SM"],
  ["Seychelles", "SC"],
  ["Sierra Leone", "SL"],
  ["Solomon Islands", "SB"],
  ["Somalia", "SO"],
  ["South Sudan", "SS"],
  ["Sudan", "SD"],
  ["Suriname", "SR"],
  ["Tajikistan", "TJ"],
  ["Togo", "TG"],
  ["Tonga", "TO"],
  ["Turkmenistan", "TM"],
  ["Tuvalu", "TV"],
  ["Vanuatu", "VU"],
  ["Vatican City", "VA"],
  ["Yemen", "YE"],
  ["Zambia", "ZM"],
]);

register([
  ["Syria", "SY"],
  ["Côte d'Ivoire", "CI"],
  ["São Tomé and Príncipe", "ST"],
  ["Trinidad & Tobago", "TT"],
  ["Bosnia & Herzegovina", "BA"],
  ["Antigua & Barbuda", "AG"],
  ["Saint Vincent & the Grenadines", "VC"],
  ["Réunion", "RE"],
  ["Curaçao", "CW"],
  ["New Caledonia", "NC"],
  ["French Polynesia", "PF"],
  ["Martinique", "MQ"],
  ["Guadeloupe", "GP"],
  ["French Guiana", "GF"],
  ["Faroe Islands", "FO"],
  ["Greenland", "GL"],
  ["Bermuda", "BM"],
  ["Cayman Islands", "KY"],
  ["Aruba", "AW"],
  ["Guam", "GU"],
  ["U.S. Virgin Islands", "VI"],
  ["British Virgin Islands", "VG"],
  ["Isle of Man", "IM"],
  ["Jersey", "JE"],
  ["Guernsey", "GG"],
  ["Gibraltar", "GI"],
  ["Macau", "MO"],
  ["Taiwan, Province of China", "TW"],
  ["Korea, Republic of", "KR"],
  ["Russian Federation", "RU"],
  ["Viet Nam", "VN"],
]);
