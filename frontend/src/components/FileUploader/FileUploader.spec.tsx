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

import { afterEach, describe, expect, rstest, test } from '@rstest/core';
import { cleanup, fireEvent, render, screen } from '@testing-library/react/pure';
import { IntlProvider } from 'react-intl';

import { FileUploader } from './FileUploader';

const ID = 'uploader';

function renderUploader(onChange = rstest.fn()) {
    render(<IntlProvider locale="en" children={<FileUploader id={ID} accept={['.nar']} onChange={onChange} />} />);
    return onChange;
}

function element(id: string): HTMLElement {
    const found = document.getElementById(id);
    if (!found) throw new Error(`BUG: #${id} is rendered`);
    return found;
}

afterEach(cleanup);

describe('FileUploader', () => {
    test('a picked file is handed over and shown with its size', () => {
        const onChange = renderUploader();
        const file = new File(['x'.repeat(1500)], 'hello.nar');

        fireEvent.change(element(`${ID}-input`), { target: { files: [file] } });

        expect(onChange).toHaveBeenLastCalledWith(file);
        expect(element(ID).parentElement?.textContent).toContain('hello.nar');
        expect(element(ID).parentElement?.textContent).toContain('1.5 kB');
    });

    test('an extension is matched regardless of case', () => {
        const onChange = renderUploader();
        const file = new File(['nar'], 'HELLO.NAR');

        fireEvent.change(element(`${ID}-input`), { target: { files: [file] } });

        expect(onChange).toHaveBeenLastCalledWith(file);
    });

    test('a dropped file of another type is shown as invalid and never handed over', () => {
        const onChange = renderUploader();

        fireEvent.drop(element(ID), { dataTransfer: { files: [new File(['zip'], 'hello.zip')] } });

        expect(onChange).toHaveBeenLastCalledWith(null);
        expect(screen.getByText('Invalid file type')).toBeTruthy();
    });

    test('removing the file withdraws it', () => {
        const onChange = renderUploader();
        fireEvent.change(element(`${ID}-input`), { target: { files: [new File(['nar'], 'hello.nar')] } });

        fireEvent.click(element(`${ID}-remove`));

        expect(onChange).toHaveBeenLastCalledWith(null);
        expect(element(ID).parentElement?.textContent).not.toContain('hello.nar');
    });

    test('clicking the drop zone opens the file chooser once', () => {
        renderUploader();
        const open = rstest.spyOn(element(`${ID}-input`), 'click');

        fireEvent.click(screen.getByRole('button', { name: 'Select a File' }));

        expect(open).toHaveBeenCalledTimes(1);
    });

    test('a file dropped beside the zone is neither opened by the browser nor handed over', () => {
        const onChange = renderUploader();
        const files = { types: ['Files'], files: [new File(['nar'], 'hello.nar')] };

        const opened = fireEvent.drop(document.body, { dataTransfer: files });

        expect(opened, 'the browser would navigate away to the file').toBe(false);
        expect(onChange).not.toHaveBeenCalled();
    });

    test('drops beside the zone are left alone once it is gone', () => {
        renderUploader();
        cleanup();

        const opened = fireEvent.drop(document.body, { dataTransfer: { types: ['Files'], files: [] } });

        expect(opened, 'the page must not keep refusing drops after the dialog closes').toBe(true);
    });

    test('dragging text beside the zone is left alone', () => {
        renderUploader();

        const dropped = fireEvent.drop(document.body, { dataTransfer: { types: ['text/plain'], files: [] } });

        expect(dropped, 'only files would replace the page').toBe(true);
    });
});
