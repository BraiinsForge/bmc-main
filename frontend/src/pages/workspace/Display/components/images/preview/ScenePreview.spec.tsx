import { beforeEach, describe, expect, test } from '@rstest/core';
import { cleanup, render } from '@testing-library/react/pure';
import * as pb from '@/proto';
import { ScenePreview } from '.';

beforeEach(cleanup);

const manifest = (iconUrl?: string) => pb.create(pb.WidgetManifestSchema, { uid: 'w', name: 'W', iconUrl });

describe('ScenePreview', () => {
    test('renders the manifest icon when iconUrl is present', () => {
        const { container } = render(<ScenePreview kind={{ manifest: manifest('/widgets/w/icon') }} />);
        expect(container.querySelector('img')?.getAttribute('src')).toBe('/widgets/w/icon');
    });

    test('falls back to the generic glyph (no img) when iconUrl is absent', () => {
        const { container } = render(<ScenePreview kind={{ manifest: manifest() }} />);
        expect(container.querySelector('img')).toBeNull();
    });

    test('combined scenes render the generic glyph, not an img', () => {
        const { container } = render(<ScenePreview kind="combined" />);
        expect(container.querySelector('img')).toBeNull();
    });
});
