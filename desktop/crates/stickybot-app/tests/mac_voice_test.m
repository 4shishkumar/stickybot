// Standalone Cocoa test. See desktop/scripts/test-mac-voice.sh. Uses only synthesized audio.
#import "../src/mac_voice.m"
#define CHECK(expr) do { if (!(expr)) { fprintf(stderr, "Failed at line %d: %s\n", __LINE__, #expr); exit(1); } } while (0)

static int begins, cancels, sends;
int sb_voice_begin(void) { begins++; return 1; }
void sb_voice_audio(const float *pcm, size_t n) { sends++; }
void sb_voice_cancel(void) { cancels++; }
void sb_voice_dashboard(void) {}

static AVAudioPCMBuffer *stereo(AVAudioFrameCount count) {
    AVAudioFormat *format = [[AVAudioFormat alloc] initStandardFormatWithSampleRate:48000 channels:2];
    AVAudioPCMBuffer *buffer = [[AVAudioPCMBuffer alloc] initWithPCMFormat:format frameCapacity:count];
    buffer.frameLength = count;
    for (AVAudioFrameCount i = 0; i < count; ++i) {
        buffer.floatChannelData[0][i] = 0;
        buffer.floatChannelData[1][i] = .4 * sin(i * 2 * M_PI * 440 / 48000);
    }
    return buffer;
}
static void testAudio(void) {
    AVAudioPCMBuffer *input = stereo(4800);
    AVAudioFormat *mono = [[AVAudioFormat alloc] initStandardFormatWithSampleRate:SBRate channels:1];
    AVAudioConverter *converter = [[AVAudioConverter alloc] initFromFormat:input.format toFormat:mono];
    converter.downmix = YES;
    SBRecording *rec = [SBRecording new];
    NSMutableData *samples = [NSMutableData data];
    [rec setValue:samples forKey:@"samples"];
    [rec setValue:converter forKey:@"converter"];
    for (int i=0; i<10; ++i) [rec append:input];
    CHECK(rec.level > 0); // the right channel reached the mono buffer
    CHECK([rec finish].length / sizeof(float) == 16000);
    CHECK(!rec.failure);
    [rec append:input];
    CHECK(samples.length / sizeof(float) == 16000);
    SBRecording *bounded = [SBRecording new];
    [converter reset];
    samples.length = SBMaxSeconds * SBRate * sizeof(float);
    [bounded setValue:samples forKey:@"samples"];
    [bounded setValue:converter forKey:@"converter"];
    [bounded append:input];
    CHECK(bounded.failure && [bounded finish].length == SBMaxSeconds * SBRate * sizeof(float));
}

static void testPanel(void) {
    [NSApplication sharedApplication];
    [NSApp setActivationPolicy:NSApplicationActivationPolicyAccessory];
    pid_t foreground = NSWorkspace.sharedWorkspace.frontmostApplication.processIdentifier;
    SBPanel *panel = [SBPanel new];
    for (NSString *phase in @[@"listening", @"transcribing", @"thinking", @"approval", @"answering", @"reply", @"error"]) {
        [panel showPhase:phase heard:@"How much space is free?" detail:@"208 GB free"];
        [panel setMicLevel:.4f];
        CHECK(panel.level == NSFloatingWindowLevel);
        CHECK(panel.isVisible && !panel.canBecomeKeyWindow && !panel.canBecomeMainWindow && !panel.keyWindow);
        NSImageView *logo = [panel valueForKey:@"logo"];
        NSTextField *title = [panel valueForKey:@"_title"];
        NSLevelIndicator *meter = [panel valueForKey:@"meter"];
        CHECK(logo.image == NSApp.applicationIconImage && NSWidth(logo.frame)==46 && title.font.pointSize==13);
        CHECK(meter.hidden != [phase isEqual:@"listening"]);
        CHECK(logo.layer.shadowOpacity > 0 || [phase isEqual:@"reply"] || [phase isEqual:@"error"]);
        if ([phase isEqual:@"reply"] || [phase isEqual:@"error"]) {
            CHECK(![logo.layer animationForKey:@"voice-pulse"] && logo.layer.shadowOpacity == 0);
        } else if (!NSWorkspace.sharedWorkspace.accessibilityDisplayShouldReduceMotion) {
            CHECK([logo.layer animationForKey:@"voice-pulse"]);
        }
        if ([phase isEqual:@"transcribing"]) {
            CHECK([title.stringValue isEqual:@"Listening"]);
            CHECK([[[panel valueForKey:@"text"] string] containsString:@"Microphone off"]);
        }
    }
    NSString *longAnswer = [@"The full answer has several paragraphs.\n" stringByPaddingToLength:4200
                         withString:@"Another paragraph.\n" startingAtIndex:0];
    [panel showPhase:@"reply" heard:@"question" detail:longAnswer];
    CHECK([[[panel valueForKey:@"text"] string] isEqual:longAnswer]);
    CHECK([[panel valueForKey:@"scroll"] hasVerticalScroller]);
    CHECK([[panel valueForKey:@"dashboard"] isHidden] == NO);
    CHECK(foreground == NSWorkspace.sharedWorkspace.frontmostApplication.processIdentifier);
    [panel orderOut:nil];
}

static void testShortcutConflict(void) {
    // Register two deliberately unlikely chords in this test process, reserving one
    // as another client. Keep the previous working registration when rebinding fails.
    const UInt32 mods = controlKey | shiftKey | optionKey;
    SBVoice *controller = [SBVoice new];
    CHECK(![controller registerCode:kVK_F17 modifiers:mods]);
    CHECK(![controller registerCode:kVK_F17 modifiers:mods]);
    EventHotKeyRef conflict = NULL;
    EventHotKeyID foreign = {'TstV',42};
    CHECK(RegisterEventHotKey(kVK_F18, mods, foreign, GetApplicationEventTarget(), kEventHotKeyExclusive, &conflict) == noErr);
    CHECK([controller registerCode:kVK_F18 modifiers:mods] != nil);
    CHECK([[controller valueForKey:@"code"] unsignedIntegerValue] == kVK_F17);
    CHECK(UnregisterEventHotKey(conflict) == noErr);
    CHECK(![controller registerCode:kVK_F18 modifiers:mods]);
    CHECK([[controller valueForKey:@"code"] unsignedIntegerValue] == kVK_F18);
    [controller shutdown];
}

static void testCancellation(void) {
    SBVoice *controller = [SBVoice new];
    [controller setValue:[SBRecording new] forKey:@"recording"];
    [controller keyEvent:SBEscapeID pressed:YES];
    CHECK(cancels == 1 && sends == 0 && begins == 0);
    [controller setValue:[SBRecording new] forKey:@"recording"];
    [controller suspend:[NSNotification notificationWithName:@"com.apple.screenIsLocked" object:nil]];
    [controller suspend:[NSNotification notificationWithName:NSWorkspaceWillSleepNotification object:nil]];
    CHECK(cancels == 2 && sends == 0);
    [controller resume:[NSNotification notificationWithName:NSWorkspaceDidWakeNotification object:nil]];
    CHECK([[controller valueForKey:@"suspended"] unsignedIntegerValue] != 0);
    [controller resume:[NSNotification notificationWithName:@"com.apple.screenIsUnlocked" object:nil]];
    CHECK([[controller valueForKey:@"suspended"] unsignedIntegerValue] == 0);
    [controller shutdown];
}
int main(void) {
    @autoreleasepool {
        @try {
            testAudio();
            testPanel();
            testShortcutConflict();
            testCancellation();
            puts("Mac voice audio, popup, focus and cancellation checks passed");
        } @catch (NSException *exception) {
            fprintf(stderr, "Native test failed: %s\n", exception.reason.UTF8String);
            return 1;
        }
    }
}
