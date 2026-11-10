// Copyright (C) 2026  Braiins Forge s.r.o.
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

import { action } from 'storybook/actions';
import type { Meta } from '@storybook/react';
import { useIntl } from 'react-intl';
import cn from 'clsx';

import * as pb from '@/proto';
import { LanguageSwitchView } from '@/pages/init/components/LanguageSwitch';
import gbFlag from '@assets/intl/english/gb.svg?raw';
import zhFlag from '@assets/intl/profile-languages/zh/flag.svg?raw';

import { LanguagePicker } from '../LanguagePicker';
import { LanguageName, LanguageUnavailable } from './LanguageName';
import css from './LanguageName.stories.scss';

/** A flag the firmware ships, as a pack hands it to the web. */
function flag(svg: string): pb.Icon {
    return pb.create(pb.IconSchema, { mimeType: 'image/svg+xml', data: btoa(svg) });
}

const english = pb.create(pb.LanguageInfoSchema, { tag: 'en-GB', nativeName: 'English (UK)', flag: flag(gbFlag) });
const chinese = pb.create(pb.LanguageInfoSchema, { tag: 'zh', nativeName: '简体中文', flag: flag(zhFlag) });
const languages = [english, chinese];

/** The settings picker as the device reads it: `null` once it no longer offers the language it is set to. */
function Picker({ id, value }: { id: string; value: null | string }) {
    const field = { value, items: languages, unavailable: value == null, onChange: action('onChange') };
    return <LanguagePicker id={id} field={field} />;
}

// Logs the pick instead of saving it on a device.
async function pick(tag: string): Promise<boolean> {
    action('onPick')(tag);
    return true;
}

/** The setup header switch speaking `tag`. A page without a listed language falls back to `en`. */
function Switch({ tag }: { tag: string }) {
    const intl = useIntl();
    return (
        <LanguageSwitchView
            intl={intl}
            offered
            languages={languages}
            tag={tag}
            isSaving={false}
            onErrors={action('onErrors')}
            onPick={pick}
        />
    );
}

/**
 * A captioned sample.
 * `hug` keeps it at its own width, as the setup header does,
 * where a settings field fills its cell.
 */
function Cell({ caption, hug, children }: { caption: string; hug?: boolean; children: ReactNode }) {
    return (
        <figure className={cn(css.cell, hug && css.hug)}>
            <figcaption className={css.caption} children={caption} />
            {children}
        </figure>
    );
}

export default {
    title: 'components/LanguageName',
} satisfies Meta;

export function EveryPicker() {
    return (
        <div className={css.grid}>
            <h3 className={css.title}>Settings picker</h3>
            <Cell caption="A listed language" children={<Picker id="listed" value="en-GB" />} />
            <Cell caption="A language no longer available" children={<Picker id="unavailable" value={null} />} />

            <h3 className={css.title}>Setup header switch</h3>
            <Cell caption="A listed language" hug children={<Switch tag="en-GB" />} />
            <Cell caption="A language no longer available" hug children={<Switch tag="en" />} />

            <h3 className={css.title}>What a picker draws for a language</h3>
            <Cell caption="LanguageName" children={<LanguageName language={english} />} />
            <Cell caption="LanguageName, flag only" children={<LanguageName language={english} flagOnly />} />
            <Cell caption="LanguageUnavailable" children={<LanguageUnavailable />} />
            <Cell caption="LanguageUnavailable, flag only" children={<LanguageUnavailable flagOnly />} />
        </div>
    );
}
