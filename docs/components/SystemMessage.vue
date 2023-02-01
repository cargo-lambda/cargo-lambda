<script>
export default {
    data() {
        return {
            win: false,
            mac: false,
            linux: false,
            fallback: true,
        }
    },
    beforeMount() {
        const platform = (navigator?.userAgentData?.platform || navigator?.platform || '').toLowerCase();
        this.win = platform.includes('win');
        this.mac = platform.includes('mac');
        this.linux = platform.includes('linux');
        this.fallback = !this.win && !this.mac && !this.linux;
    }
}
</script>

<template>
    <div v-if="win">
        <slot name="win"></slot>
    </div>

    <div v-if="mac">
        <slot name="mac"></slot>
    </div>

    <div v-if="linux">
        <slot name="linux"></slot>
    </div>

    <div v-if="fallback">
        <slot name="fallback"></slot>
    </div>
</template>