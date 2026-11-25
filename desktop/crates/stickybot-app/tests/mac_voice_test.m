// Standalone Cocoa test. See desktop/scripts/test-mac-voice.sh. Uses only synthesized audio.
#import "../src/mac_voice.m"
#define CHECK(expr) do { if (!(expr)) { fprintf(stderr, "Failed at line %d: %s\n", __LINE__, #expr); exit(1); } } while (0)

static int begins, cancels, sends, turnCancels;
int sb_voice_begin(void) { begins++; return 1; }
void sb_voice_audio(const float *pcm, size_t n) { sends++; }
void sb_voice_cancel(void) { cancels++; }
void sb_voice_cancel_turn(void) { turnCancels++; }
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
    CHECK(rec.level > 0 && rec.peak >= rec.level); // the right channel reached the mono buffer
    CHECK(fabs(rec.seconds - 1.0) < .02);
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
        SBWave *wave = [panel valueForKey:@"wave"];
        NSButton *close = [panel valueForKey:@"cancel"];
        CHECK(logo.image == NSApp.applicationIconImage && NSWidth(logo.frame)==46 && title.font.pointSize==13);
        CHECK(wave.hidden != [phase isEqual:@"listening"]);
        CHECK(close.hidden == [phase isEqual:@"listening"]); // Esc cancels while listening, the button afterwards
        CHECK(logo.layer.shadowOpacity > 0 || [phase isEqual:@"reply"] || [phase isEqual:@"error"]);
        if ([phase isEqual:@"reply"] || [phase isEqual:@"error"]) {
            CHECK(![logo.layer animationForKey:@"voice-pulse"] && logo.layer.shadowOpacity == 0);
        } else if (!NSWorkspace.sharedWorkspace.accessibilityDisplayShouldReduceMotion) {
            CHECK([logo.layer animationForKey:@"voice-pulse"]);
        }
        if ([phase isEqual:@"transcribing"]) CHECK([title.stringValue isEqual:@"Transcribing"]);
    }
    // The cancel button stops a running request; after the answer it only dismisses.
    [panel showPhase:@"thinking" heard:@"q" detail:@""];
    [[panel valueForKey:@"cancel"] performClick:nil];
    CHECK(turnCancels == 1 && !panel.isVisible);
    [panel showPhase:@"reply" heard:@"q" detail:@"a"];
    [[panel valueForKey:@"cancel"] performClick:nil];
    CHECK(turnCancels == 1 && !panel.isVisible);
    // A permission error offers System Settings.
    [panel showPhase:@"error" heard:@"" detail:SBMicDenied];
    CHECK([[[panel valueForKey:@"action"] title] isEqual:@"Open Privacy Settings"] && ![[panel valueForKey:@"action"] isHidden]);
    NSString *longAnswer = [@"The full answer has several paragraphs.\n" stringByPaddingToLength:4200
                         withString:@"Another paragraph.\n" startingAtIndex:0];
    [panel showPhase:@"reply" heard:@"question" detail:longAnswer];
    CHECK([[[panel valueForKey:@"text"] string] isEqual:longAnswer]);
    CHECK([[panel valueForKey:@"scroll"] hasVerticalScroller]);
    CHECK([[panel valueForKey:@"action"] isHidden] == NO);
    CHECK([[[panel valueForKey:@"action"] title] isEqual:@"Open dashboard"]);
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

static void testEscapeClosesThePanel(void) {
    // Esc after an answer closes the panel and gives Esc back; Esc during a request cancels it.
    SBVoice *controller = [SBVoice new];
    [controller show:@"reply" heard:@"q" detail:@"a"];
    SBPanel *panel = [controller valueForKey:@"panel"];
    CHECK(panel.isVisible);
    [controller keyEvent:SBEscapeID pressed:YES];
    CHECK(!panel.isVisible && turnCancels == 1);
    [controller setValue:@YES forKey:@"pending"];
    [controller show:@"thinking" heard:@"q" detail:@""];
    [controller keyEvent:SBEscapeID pressed:YES];
    CHECK(!panel.isVisible && turnCancels == 2);
    [controller shutdown];
}

static void testInputs(void) {
    // The device list is valid JSON, and an unknown saved microphone falls back to the default.
    sb_voice_set_input("no-such-device-uid");
    NSDictionary *inputs = [NSJSONSerialization JSONObjectWithData:[[NSString stringWithUTF8String:sb_voice_inputs()]
        dataUsingEncoding:NSUTF8StringEncoding] options:0 error:nil];
    CHECK([inputs[@"devices"] isKindOfClass:NSArray.class]);
    CHECK([inputs[@"selected"] isEqual:@"no-such-device-uid"] && [inputs[@"fallback"] boolValue]);
    CHECK(SBChosenInput() == SBDefaultInput());
    sb_voice_set_input("");
    inputs = [NSJSONSerialization JSONObjectWithData:[[NSString stringWithUTF8String:sb_voice_inputs()]
        dataUsingEncoding:NSUTF8StringEncoding] options:0 error:nil];
    CHECK([inputs[@"selected"] isEqual:@""] && ![inputs[@"fallback"] boolValue]);
}

static void testSilence(void) {
    // A recording that never got louder than the silence floor is not sent.
    SBVoice *controller = [SBVoice new];
    SBRecording *quiet = [SBRecording new];
    [quiet setValue:[NSMutableData dataWithLength:16000 * sizeof(float)] forKey:@"samples"];
    [controller setValue:quiet forKey:@"recording"];
    int before = sends, cancelled = cancels;
    [controller finishRecording:YES error:nil];
    CHECK(sends == before && cancels == cancelled + 1);
    SBPanel *panel = [controller valueForKey:@"panel"];
    CHECK([[[panel valueForKey:@"text"] string] hasPrefix:@"Didn't hear anything"]);
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
            testInputs();
            testSilence();
            testEscapeClosesThePanel();
            puts("Mac voice audio, popup, focus and cancellation checks passed");
        } @catch (NSException *exception) {
            fprintf(stderr, "Native test failed: %s\n", exception.reason.UTF8String);
            return 1;
        }
    }
}
