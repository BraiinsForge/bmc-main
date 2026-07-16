// Copyright (C) 2025  Braiins Systems s.r.o.
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.
//
// Braiins Systems s.r.o. and Braiins Forge s.r.o. each reserve the right
// to grant any party a license to this program, or any part thereof,
// under any terms, and such a grant shall be considered distinct from
// the grant above.

const $ = {
    // The gray scale below reuses the Carbon Design System gray palette.
    // Copyright IBM Corp., Carbon Design System, licensed under Apache-2.0.
    gray10: '#f4f4f4',
    gray20: '#e0e0e0',
    gray30: '#c6c6c6',
    gray40: '#a8a8a8',
    gray50: '#8d8d8d',
    gray60: '#6f6f6f',
    gray70: '#525252',
    gray80: '#393939',
    gray90: '#262626',
    gray100: '#161616',

    lime10: '#e0fcd6',
    lime20: '#a6f382',
    lime30: '#89db5d',
    lime40: '#6dbc39',
    lime50: '#599f2a',
    lime60: '#457d1f',
    lime70: '#355c15',
    lime80: '#26400c',
    lime90: '#182b05',
    lime100: '#081905',

    green10: '#ddfbe9',
    green20: '#a3f1b9',
    green30: '#5adf88',
    green40: '#34c06a',
    green50: '#13a454',
    green60: '#168042',
    green70: '#195e33',
    green80: '#124223',
    green90: '#102b19',
    green100: '#061912',

    teal10: '#e0fafb',
    teal20: '#a3ecf1',
    teal30: '#56d8e0',
    teal40: '#00bac5',
    teal50: '#009da7',
    teal60: '#007c83',
    teal70: '#005e5e',
    teal80: '#004042',
    teal90: '#002a2d',
    teal100: '#031a1c',

    blue10: '#ecf5ff',
    blue20: '#d0e0ff',
    blue30: '#a9c7ff',
    blue40: '#7ca8ff',
    blue50: '#4b8aff',
    blue60: '#2460ff',
    blue70: '#1043cd',
    blue80: '#0a2e9b',
    blue90: '#071d67',
    blue100: '#071338',

    violet10: '#f3f2ff',
    violet20: '#dfdcff',
    violet30: '#c5c0ff',
    violet40: '#a69dff',
    violet50: '#8b7cff',
    violet60: '#6b50ff',
    violet70: '#5432cd',
    violet80: '#39238f',
    violet90: '#281661',
    violet100: '#170d3a',

    purple10: '#fbf1fb',
    purple20: '#f2d6fd',
    purple30: '#e3b6fa',
    purple40: '#d28df7',
    purple50: '#c063f9',
    purple60: '#a72dea',
    purple70: '#7e1cb2',
    purple80: '#59137d',
    purple90: '#3b1151',
    purple100: '#200f29',

    magenta10: '#fff0f6',
    magenta20: '#ffd5e4',
    magenta30: '#ffb0ca',
    magenta40: '#fb82a8',
    magenta50: '#ee5884',
    magenta60: '#d3265d',
    magenta70: '#a01743',
    magenta80: '#720f2d',
    magenta90: '#4f071d',
    magenta100: '#290c17',

    red10: '#fff1f2',
    red20: '#ffd6d5',
    red30: '#ffb3b2',
    red40: '#ff8384',
    red50: '#f95355',
    red60: '#d9222c',
    red70: '#a2171f',
    red80: '#740e14',
    red90: '#4f090d',
    red100: '#2b0b0b',

    orange10: '#fff1e9',
    orange20: '#ffd8bf',
    orange30: '#ffb687',
    orange40: '#fe8431',
    orange50: '#eb6307',
    orange60: '#c14812',
    orange70: '#933200',
    orange80: '#642600',
    orange90: '#421b00',
    orange100: '#251200',

    gold10: '#fff2de',
    gold20: '#fddc95',
    gold30: '#feba53',
    gold40: '#ed9419',
    gold50: '#cf790e',
    gold60: '#a45f09',
    gold70: '#7b4505',
    gold80: '#573002',
    gold90: '#3b1f01',
    gold100: '#241100',

    yellow10: '#fcf4d6',
    yellow20: '#fedd6f',
    yellow30: '#f4c01a',
    yellow40: '#d3a103',
    yellow50: '#b28700',
    yellow60: '#8e6b00',
    yellow70: '#694f04',
    yellow80: '#493605',
    yellow90: '#312402',
    yellow100: '#1d1401',
} as const;
const C = {
    ...$,

    accentRed: $.red50,
    accentPurple: $.purple60,
    accentViolet: $.violet60,
    accentBlue: $.blue60,
    accentTeal: $.teal30,

    alertRed: $.red60,
    alertGreen: $.green40,
    alertYellow: $.yellow30,
    alertOrange: $.orange50,
    alertBlue: $.blue50,
    alertViolet: $.violet60,
} as const;

export default C;

export const social = {
    facebook: '#2d88ff',
    instagram: '#cd2f94',
    linkedin: '#0a66c2',
    telegram: '#32afed',
    twitter: '#58E5F9',
    weechat: '#2cbb00',
    youtube: '#f00',
};

export { default as Color } from 'color';
