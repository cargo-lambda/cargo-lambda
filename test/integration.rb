require 'fileutils'
include FileUtils
require 'test/unit/assertions'
include Test::Unit::Assertions

BINARY = File.expand_path(File.join('target', 'debug', 'cargo-lambda'))
BASE = File.expand_path(File.join('test', 'integration'))

rm_rf BASE
mkdir_p BASE

def test_build(name, output, new_flags: '', build_flags: '')
    cd BASE
    system "#{BINARY} lambda new #{new_flags} #{name}"

    cd name
    system "#{BINARY} lambda build --release --quiet #{build_flags}"
    output = File.join('target', 'lambda', output)
    assert(File.exist?(output), "binary doesn't exist: #{output}")
end

def test_version
    regex = %r{cargo-lambda \d+\.\d+\.\d+(-pre\d+)? \([a-zA-Z0-9]+(-dirty)? \d{4}-\d{2}-\d{2}Z\)}
    out = `#{BINARY} lambda --version`.chomp
    assert(out =~ regex, "version doesn't match: `#{out}`, `#{regex}")
end

puts "testing version output"
test_version

puts "testing HTTP functions"
test_build('test-fun', 'test-fun', new_flags: '--http')

puts "testing basic extensions"
test_build('test-ext', File.join('extensions', 'test-ext'), 
    new_flags: '--extension', build_flags: '--extension')

puts "testing logs extensions"
test_build('test-logs', File.join('extensions', 'test-logs'), 
    new_flags: '--extension --logs', build_flags: '--extension')
