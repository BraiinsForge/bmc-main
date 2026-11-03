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

import { Component } from 'react';
import { useIntl, type IntlShape } from 'react-intl';
import cn from 'clsx';

import * as pb from '@/proto';
import { getID } from '@/lib/form';
import { store, useStore } from '@/store';

import { Dropdown, LanguageName } from '@/components';
import type { DropdownProps } from '@carbon/react';

import css from './LanguageSwitch.scss';

export interface LanguageSwitchProps {
    /** `null` clears what an earlier pick reported. */
    onErrors(errors: null | string[]): void;
    className?: string;
}
interface Props extends LanguageSwitchProps {
    intl: IntlShape;
    offered: boolean;
    languages: ReadonlyArray<pb.LanguageInfo>;
    /** The device's language, which differs from the page's after a pick the page could not follow. */
    tag: string;
    isSaving: boolean;
}

const $ = getID('initial-setup').get;

/** A language the device does not list, named in itself as the browser knows it. */
function unlisted(tag: string): pb.LanguageInfo {
    let nativeName = tag;
    try {
        nativeName = new Intl.DisplayNames([tag], { type: 'language' }).of(tag) ?? tag;
    } catch {
        // A malformed tag keeps its own spelling.
    }
    return pb.create(pb.LanguageInfoSchema, { tag, nativeName });
}

class View extends Component<Props> {
    // A list that fails to load hides the switch and leaves the setup in its current language.
    componentDidMount() {
        if (this.props.offered) void store.loadLanguages();
    }

    // Not aborted on unmount: that cannot undo a save the device already made,
    // and the store still speaks the pick on the next setup page.
    #change: DropdownProps<null | pb.LanguageInfo>['onChange'] = async ({ selectedItem }) => {
        const { intl, onErrors, tag } = this.props;
        if (!selectedItem || selectedItem.tag === tag || store.isSavingLanguage) return;

        onErrors(null);
        try {
            const spoken = await store.setSetupLanguage(selectedItem.tag);
            if (!spoken) {
                onErrors([
                    intl.formatMessage({
                        defaultMessage: 'Language saved; reload the page to see it',
                        description:
                            'Setup error when the device saved the new language but the page could not load its translation; the page stays in its current language until reloaded',
                    }),
                ]);
            }
        } catch ($) {
            // Neither the browser's text for a lost connection
            // nor the device's refusal speaks the page's language.
            console.warn('Language not saved', $);
            onErrors([
                intl.formatMessage({
                    defaultMessage: 'Failed to change the language',
                    description:
                        'Setup error, shown alone without its cause, when the device did not save the language picked in the setup header',
                }),
            ]);
        }
    };

    #toString = (x: null | pb.LanguageInfo): string => x?.nativeName ?? '';
    #renderElement = (x: null | pb.LanguageInfo): NonNullable<ReactNode> => (x ? <LanguageName language={x} /> : '');
    // Flag only, like the device's own switch, so the header keeps room for the logo.
    #renderSelected = (x: null | pb.LanguageInfo): NonNullable<ReactNode> =>
        x ? <LanguageName language={x} flagOnly /> : '';

    render() {
        const { intl, offered, languages, tag, isSaving, className } = this.props;
        if (!offered || languages.length < 2) return null;
        // A page booted in a language the device no longer lists keeps it as the selected entry
        // until another is picked, so the switch never shows empty.
        const listed = languages.find(x => x.tag === tag);
        const selected = listed ?? unlisted(tag);
        const items = listed ? Array.from(languages) : [selected, ...languages];

        return (
            <Dropdown<null | pb.LanguageInfo>
                id={$('language')}
                className={cn(css.root, className)}
                size="sm"
                label=""
                titleText={intl.formatMessage({
                    defaultMessage: 'Language',
                    description: 'Accessible name of the language switch in the setup header',
                })}
                hideLabel
                items={items}
                selectedItem={selected}
                onChange={this.#change}
                itemToString={this.#toString}
                itemToElement={this.#renderElement}
                renderSelectedItem={this.#renderSelected}
                // Not `disabled`, which would drop the focus of whoever just picked.
                readOnly={isSaving}
            />
        );
    }
}

export function LanguageSwitch(props: LanguageSwitchProps) {
    const intl = useIntl();
    const offered = useStore(x => x.i18n);
    const languages = useStore(x => x.languages);
    const tag = useStore(x => x.unspokenLanguage ?? x.locale.tag);
    const isSaving = useStore(x => x.isSavingLanguage);
    return <View {...props} intl={intl} offered={offered} languages={languages} tag={tag} isSaving={isSaving} />;
}
