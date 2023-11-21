const toml = require('toml');
const fs = require('fs');

const meta = toml.parse(fs.readFileSync('../Cargo.toml', 'utf-8'));
const version = meta['workspace']['package']['version'];

export default {
    lang: 'en-US',
    title: 'Cargo Lambda',
    description: 'Rust functions on AWS Lambda made simple',

    lastUpdated: true,

    themeConfig: {
        nav: nav(),

        sidebar: {
            '/guide/': sidebar(),
            '/commands/': sidebar()
        },

        editLink: {
            pattern: 'https://github.com/cargo-lambda/cargo-lambda/edit/main/docs/:path',
            text: 'Edit this page on GitHub'
        },

        socialLinks: [
            { icon: 'github', link: 'https://github.com/cargo-lambda/cargo-lambda' }
        ],

        footer: {
            message: 'Released under the MIT License.',
            copyright: 'Copyright © 2022-present David Calavera'
        },

        search: {
            provider: 'local'
        }
    },
};

function nav() {
    return [
        { text: 'Guide', link: '/guide/what-is-cargo-lambda', activeMatch: '/guide/' },
        { text: 'Commands', link: '/commands/introduction', activeMatch: '/commands/' },
        { text: `Version: ${version}`, link: `https://github.com/cargo-lambda/cargo-lambda/releases/tag/v${version}` }
    ]
}

function sidebar() {
    return [
        {
            text: 'Introduction',
            collapsible: true,
            items: [
                { text: 'What is Cargo Lambda?', link: '/guide/what-is-cargo-lambda' },
                { text: 'Installation', link: '/guide/installation' },
                { text: 'Getting Started', link: '/guide/getting-started' },
                { text: 'Cross Compiling', link: '/guide/cross-compiling' },
                { text: 'Release Optimizations', link: '/guide/release-optimizations' },
                { text: 'Lambda Extensions', link: '/guide/lambda-extensions' },
                { text: 'Automating deployments', link: '/guide/automating-deployments' },
                { text: 'Screencasts', link: '/guide/screencasts' },
            ]
        },
        {
            text: 'Commands',
            collapsible: true,
            items: [
                { text: 'Supported commands', link: '/commands/introduction' },
                { text: 'cargo lambda build', link: '/commands/build' },
                { text: 'cargo lambda deploy', link: '/commands/deploy' },
                { text: 'cargo lambda init', link: '/commands/init' },
                { text: 'cargo lambda invoke', link: '/commands/invoke' },
                { text: 'cargo lambda new', link: '/commands/new' },
                { text: 'cargo lambda watch', link: '/commands/watch' },
            ]
        }
    ]
}
