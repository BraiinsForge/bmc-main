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

import { useState } from 'react';
import { FormattedMessage, useIntl } from 'react-intl';

import css from './InstallWidgetPackageModal.scss';

import { URLS } from '@/constants';
import { FileUploader, Link, Modal } from '@/components';

export interface InstallWidgetPackageModalProps {
    id: string;
    installing: boolean;
    onClose(): void;
    onInstall(file: File): void;
}

/** The third-party risk, stated where the package is chosen. */
export function InstallWidgetPackageModal({ id, installing, onClose, onInstall }: InstallWidgetPackageModalProps) {
    const { formatMessage } = useIntl();
    const [file, setFile] = useState<null | File>(null);

    return (
        <Modal
            id={id}
            open
            danger
            size="sm"
            modalHeading={formatMessage({ defaultMessage: 'Install a Third-Party Widget' })}
            primaryButtonText={formatMessage({ defaultMessage: 'Accept the Risk and Install' })}
            primaryButtonDisabled={!file}
            onRequestSubmit={() => file && onInstall(file)}
            loadingStatus={installing ? 'active' : 'inactive'}
            loadingDescription={formatMessage({ defaultMessage: 'Installing…' })}
            secondaryButtonText={formatMessage({ defaultMessage: 'Cancel' })}
            onSecondarySubmit={onClose}
            // Without a handler the close button is hidden.
            onRequestClose={installing ? undefined : onClose}
        >
            <div className={css.message}>
                <FormattedMessage
                    tagName="p"
                    defaultMessage="Widget packages installed from a file are not checked by Braiins Forge. A malicious package can take over this device and anything it can reach on your network. Install only packages from a source you trust."
                />
                <FormattedMessage
                    tagName="p"
                    defaultMessage="Upload the widget as a .nar archive of its package. See the <a>bmc-main repository</a> for instructions."
                    values={{ a: ch => <Link href={URLS.external.bmcMainRepository} external children={ch} /> }}
                />
            </div>
            <div inert={installing}>
                <FileUploader id={`${id}-file`} accept={['.nar']} onChange={setFile} />
            </div>
        </Modal>
    );
}
