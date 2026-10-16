// Copyright (C) 2025  Braiins Systems s.r.o.
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

import { useEffect, useRef, useState, type DragEvent } from 'react';
import { useIntl } from 'react-intl';
import { Close as IconClose } from '@carbon/react/icons';

import cn from 'clsx';
import css from './FileUploader.scss';

import { formatBytes } from '@/lib/format';
import { InlineNotification } from '../InlineNotification';

export interface FileUploaderProps {
    // Required because it's used for testing; the file input is `${id}-input`.
    id: string;
    // File extensions (`.nar`) or exact MIME types; a chosen file matching neither is shown as invalid.
    accept: string[];
    // `null` while no file is chosen or the chosen one has the wrong type.
    onChange(file: null | File): void;
}

/** A single-file drop zone, ported from the bos-main frontend's `FileUploader`. */
export function FileUploader(props: FileUploaderProps) {
    const { id, accept, onChange } = props;
    const { formatMessage } = useIntl();
    const inputRef = useRef<HTMLInputElement>(null);
    const [dragOver, setDragOver] = useState(false);
    const [chosen, setChosen] = useState<null | { file: File; invalid: boolean }>(null);

    // A file dropped beside the zone would otherwise make the browser open it in place of the page.
    useEffect(() => {
        const refuse = (e: globalThis.DragEvent): void => {
            if (e.defaultPrevented || !e.dataTransfer?.types.includes('Files')) return;
            e.preventDefault();
            e.dataTransfer.dropEffect = 'none';
        };
        document.addEventListener('dragover', refuse);
        document.addEventListener('drop', refuse);
        return () => {
            document.removeEventListener('dragover', refuse);
            document.removeEventListener('drop', refuse);
        };
    }, []);

    const choose = (file: undefined | File): void => {
        if (!file) return;
        const dot = file.name.lastIndexOf('.');
        const extension = dot < 0 ? '' : file.name.slice(dot).toLowerCase();
        const invalid = !accept.includes(extension) && !accept.includes(file.type);
        setChosen({ file, invalid });
        onChange(invalid ? null : file);
    };
    const remove = (): void => {
        setChosen(null);
        onChange(null);
    };
    const pick = (): void => {
        inputRef.current?.click();
    };
    const dragOverZone = (e: DragEvent<HTMLDivElement>): void => {
        e.preventDefault();
        setDragOver(true);
        e.dataTransfer.dropEffect = 'copy';
    };
    const drop = (e: DragEvent<HTMLDivElement>): void => {
        e.preventDefault();
        setDragOver(false);
        choose(e.dataTransfer.files[0]);
    };

    return (
        <div className={css.root}>
            {/* biome-ignore lint/a11y/useKeyWithClickEvents: The inner button takes the keyboard; a click anywhere on the zone only widens its target. */}
            <div
                id={id}
                className={cn(css.dropContainer, dragOver && css.dragOver, chosen?.invalid && css.invalid)}
                onDragOver={dragOverZone}
                onDragLeave={() => setDragOver(false)}
                onDrop={drop}
                onClick={pick}
            >
                <div
                    className={css.title}
                    children={formatMessage({ defaultMessage: 'Drag and Drop your File Here' })}
                />
                <button
                    type="button"
                    className={cn(css.button, dragOver && css.primary)}
                    children={formatMessage({ defaultMessage: 'Select a File' })}
                />
                <input
                    ref={inputRef}
                    id={`${id}-input`}
                    type="file"
                    className="cds--visually-hidden"
                    tabIndex={-1}
                    aria-hidden
                    accept={accept.join(',')}
                    // Cleared so choosing the same file again fires change again.
                    onClick={e => {
                        e.stopPropagation();
                        e.currentTarget.value = '';
                    }}
                    onChange={e => choose(e.target.files?.[0])}
                />
            </div>

            {chosen && (
                <div className={cn(css.file, chosen.invalid && css.invalid)}>
                    <div className={css.name} title={chosen.file.name}>
                        <span children={chosen.file.name} />
                        <span className={css.size} children={formatBytes(chosen.file.size)} />
                    </div>
                    <span className={css.actions}>
                        <button
                            type="button"
                            id={`${id}-remove`}
                            aria-label={formatMessage({ defaultMessage: 'Remove the file' })}
                            className={css.deleteButton}
                            onClick={remove}
                            children={<IconClose />}
                        />
                    </span>
                </div>
            )}

            {chosen?.invalid && (
                <InlineNotification
                    stretch
                    kind="error"
                    hideCloseButton
                    title={formatMessage({ defaultMessage: 'Invalid file type' })}
                    className={css.errorMessage}
                />
            )}
        </div>
    );
}
