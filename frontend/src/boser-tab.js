// Copyright (C) 2026  Braiins Systems s.r.o.
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

// Inlined into the page head by rsbuild as a module script, which defers too:
// it runs after system.js and before the bundle, so a BMM's tab replaces the Deck's title and icon
// as soon as system.js runs.
// BRAND exists only once boser's brand.js loaded, so boser is up to serve the icon.
// boser-tab.spec.ts holds these checks to `boserChrome` and `parseBrand`.
const brand = window.SYSTEM?.capabilities?.boser_managed ? window.BRAND : undefined;
if (brand) {
    // Busted like boser's own index.html does, so a rebrand doesn't keep the old icon.
    const icon = `/var/favicon.png?v=${Date.now()}`;
    for (const link of document.querySelectorAll('link[rel="icon"]')) link.href = icon;
    if (typeof brand.name === 'string' && brand.name) document.title = brand.name;
}
