module.exports = {
    extends: ['stylelint-config-standard-scss'],
    customSyntax: 'postcss-scss',
    rules: {
        'property-no-unknown': [
            true,
            {
                ignoreProperties: [
                    // CSS Modules composition
                    // https://github.com/css-modules/css-modules#composition
                    'composes',
                ],
            },
        ],
        'selector-pseudo-class-no-unknown': [
            true,
            {
                ignorePseudoClasses: [
                    // CSS Modules :global scope
                    // https://github.com/css-modules/css-modules#exceptions
                    'global',
                    'local',
                ],
            },
        ],
        'value-keyword-case': [
            'lower',
            {
                // https://github.com/css-modules/css-modules#composition
                ignoreProperties: ['composes'],
                ignoreKeywords: ['overflowHidden'],
            },
        ],
        'declaration-block-no-duplicate-properties': [
            true,
            {
                // https://github.com/css-modules/css-modules#composition
                ignoreProperties: ['composes'],
            },
        ],
        'declaration-block-no-redundant-longhand-properties': [
            true,
            { ignoreShorthands: ['/grid/', '/margin-inline/'] },
        ],

        // Sass at-rules
        'at-rule-no-unknown': null,
        'scss/at-rule-no-unknown': true,
        'no-invalid-position-at-import-rule': null,

        // We like declarations grouping
        'scss/comment-no-empty': null,
        'declaration-empty-line-before': null,
        'scss/dollar-variable-empty-line-before': null,
        'scss/double-slash-comment-empty-line-before': null,

        // This just doesn't work for us for reasons clearly stated
        // in the documentation & catches thousands of false-positives.
        //
        // TLDR:
        // It doesn't take into account namespacing selectors, which means
        // that following is considered an error while it can be absolutely fine.
        //
        //  .component1 a {}
        //  .component1 a:hover {}
        //  .component2 a {}
        'no-descending-specificity': null,

        // Insensible amount of errors & fixing them doesn't contribute much
        'selector-id-pattern': null,
        'selector-class-pattern': null,
        'rule-empty-line-before': null,
        'scss/at-mixin-pattern': null,
        'scss/dollar-variable-pattern': null,

        // Plain old non-sense
        'scss/at-rule-conditional-no-parentheses': null,

        // Formatting preferences (likely caused by Prettier)
        'scss/dollar-variable-colon-space-after': 'always-single-line',

        'media-query-no-invalid': true,
        'media-feature-name-no-unknown': true,
        'media-feature-name-value-no-unknown': true,
    },
    reportNeedlessDisables: true,
    reportInvalidScopeDisables: true,
    reportDescriptionlessDisables: true,
};
