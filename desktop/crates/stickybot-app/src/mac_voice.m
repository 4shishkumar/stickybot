// The Mac input for StickyBot. Carbon delivers only the registered chord, even
// when another app has focus. The audio engine exists only during an owned hold.
#import <AppKit/AppKit.h>
#import <AVFoundation/AVFoundation.h>
#import <Carbon/Carbon.h>
#import <CoreAudio/CoreAudio.h>
#import <QuartzCore/QuartzCore.h>

extern int sb_voice_begin(void);
extern void sb_voice_audio(const float *pcm, size_t count);
extern void sb_voice_cancel(void);
extern void sb_voice_dashboard(void);

static const NSUInteger SBRate = 16000;
static const NSUInteger SBMaxSeconds = 60;
static NSString *const SBMicDenied = @"Allow StickyBot in System Settings → Privacy & Security → Microphone, then try again.";

static AudioDeviceID SBInput(void) {
    AudioObjectPropertyAddress address = {kAudioHardwarePropertyDefaultInputDevice,
        kAudioObjectPropertyScopeGlobal, kAudioObjectPropertyElementMain};
    AudioDeviceID device = kAudioObjectUnknown;
    UInt32 size = sizeof(device);
    if (AudioObjectGetPropertyData(kAudioObjectSystemObject, &address, 0, NULL, &size, &device) != noErr)
        return kAudioObjectUnknown;
    return device;
}

const char *sb_voice_input_name(void) {
    static NSString *name;
    AudioObjectPropertyAddress address = {kAudioObjectPropertyName,
        kAudioObjectPropertyScopeGlobal, kAudioObjectPropertyElementMain};
    CFStringRef value = NULL;
    UInt32 size = sizeof(value);
    if (AudioObjectGetPropertyData(SBInput(), &address, 0, NULL, &size, &value) == noErr && value)
        name = CFBridgingRelease(value);
    else name = @"No input device";
    return name.UTF8String;
}

const char *sb_voice_permission(void) {
    switch ([AVCaptureDevice authorizationStatusForMediaType:AVMediaTypeAudio]) {
        case AVAuthorizationStatusAuthorized: return "authorized";
        case AVAuthorizationStatusDenied: return "denied";
        case AVAuthorizationStatusRestricted: return "restricted";
        default: return "undetermined";
    }
}

@interface SBRecording : NSObject
@property(nonatomic, readonly) AVAudioEngine *engine;
@property(nonatomic, readonly) AudioDeviceID device;
@property(atomic, readonly) float level;
@property(atomic, copy, readonly) NSString *failure;
- (NSString *)start;
- (NSData *)finish;
@end

@implementation SBRecording {
    AVAudioConverter *_converter;
    NSMutableData *_samples;
    BOOL _tap, _closed;
}
@synthesize level = _level, failure = _failure;
- (NSString *)start {
    _device = SBInput();
    if (_device == kAudioObjectUnknown) return @"No microphone is available. Select an input in Sound settings.";
    _samples = [NSMutableData dataWithCapacity:SBMaxSeconds * SBRate * sizeof(float)];
    NSError *error = nil;
    @try {
        _engine = [[AVAudioEngine alloc] init];
        AVAudioInputNode *input = _engine.inputNode;
        AVAudioFormat *source = [input outputFormatForBus:0];
        AVAudioFormat *mono = [[AVAudioFormat alloc] initStandardFormatWithSampleRate:SBRate channels:1];
        if (!source.channelCount || source.sampleRate <= 0) return @"The selected microphone has no audio input.";
        _converter = [[AVAudioConverter alloc] initFromFormat:source toFormat:mono];
        if (!_converter) return @"The selected microphone format is unavailable.";
        _converter.downmix = YES;
        __weak SBRecording *weakSelf = self;
        AVAudioNodeTapBlock tap = ^(AVAudioPCMBuffer *buffer, AVAudioTime *__unused time) { [weakSelf append:buffer]; };
#if __MAC_OS_X_VERSION_MAX_ALLOWED >= 270000
        if (@available(macOS 27.0, *)) {
            if (![input installTapOnBus:0 bufferSize:1024 format:source error:&error block:tap])
                return error.localizedDescription ?: @"Could not open the microphone.";
        } else
#endif
        {
#pragma clang diagnostic push
#pragma clang diagnostic ignored "-Wdeprecated-declarations" // available on the minimum macOS 13 target
            [input installTapOnBus:0 bufferSize:1024 format:source block:tap];
#pragma clang diagnostic pop
        }
        _tap = YES;
        if (![_engine startAndReturnError:&error]) return error.localizedDescription ?: @"Could not start the microphone.";
    } @catch (NSException *exception) {
        // Core Audio may withdraw the input between checking its format and installing the tap.
        return @"The microphone changed. Select an input and try again.";
    }
    return nil;
}

- (void)append:(AVAudioPCMBuffer *)buffer {
    @synchronized (self) {
        if (_closed || _failure || !buffer.frameLength) return;
        if (![_converter.inputFormat isEqual:buffer.format]) {
            _failure = @"The microphone format changed. Hold the shortcut to try again.";
            return;
        }
        AVAudioFrameCount capacity = (AVAudioFrameCount)ceil(buffer.frameLength * (double)SBRate / buffer.format.sampleRate) + 64;
        AVAudioPCMBuffer *out = [[AVAudioPCMBuffer alloc] initWithPCMFormat:_converter.outputFormat frameCapacity:capacity];
        __block BOOL consumed = NO;
        NSError *error = nil;
        AVAudioConverterOutputStatus status = [_converter convertToBuffer:out error:&error
            withInputFromBlock:^AVAudioBuffer *(AVAudioPacketCount __unused packets, AVAudioConverterInputStatus *inputStatus) {
                if (consumed) { *inputStatus = AVAudioConverterInputStatus_NoDataNow; return nil; }
                consumed = YES;
                *inputStatus = AVAudioConverterInputStatus_HaveData;
                return buffer;
            }];
        if (status == AVAudioConverterOutputStatus_Error) {
            _failure = error.localizedDescription ?: @"The microphone audio could not be read.";
            return;
        }
        if (_samples.length + out.frameLength * sizeof(float) > SBMaxSeconds * SBRate * sizeof(float)) {
            _failure = @"Recording reached one minute. Hold the shortcut again for a shorter question.";
            return;
        }
        if (!out.frameLength) return;
        const float *samples = out.floatChannelData[0];
        [_samples appendBytes:samples length:out.frameLength * sizeof(float)];
        double power = 0;
        for (AVAudioFrameCount i = 0; i < out.frameLength; ++i) power += samples[i] * samples[i];
        _level = fmin(1, sqrt(power / out.frameLength) * 6);
    }
}

// The tap runs on Core Audio's thread; the main run loop owns the engine.
- (float)level { @synchronized (self) { return _level; } }
- (NSString *)failure { @synchronized (self) { return _failure; } }
- (NSData *)finish {
    @synchronized (self) { _closed = YES; }
    [_engine stop];
    if (_tap) { [_engine.inputNode removeTapOnBus:0]; _tap = NO; }
    // The sample-rate converter retains the end of the last syllable. Drain its
    // filter after the tap stops, including when the user releases mid-buffer.
    if (_converter && !_failure) {
        AVAudioPCMBuffer *tail = [[AVAudioPCMBuffer alloc] initWithPCMFormat:_converter.outputFormat frameCapacity:1024];
        while (_samples.length < SBMaxSeconds * SBRate * sizeof(float)) {
            NSError *error = nil;
            AVAudioConverterOutputStatus status = [_converter convertToBuffer:tail error:&error
                withInputFromBlock:^AVAudioBuffer *(AVAudioPacketCount __unused n, AVAudioConverterInputStatus *s) {
                    *s = AVAudioConverterInputStatus_EndOfStream;
                    return nil;
                }];
            if (status == AVAudioConverterOutputStatus_Error) {
                _failure = error.localizedDescription ?: @"Could not finish reading the microphone.";
                break;
            }
            NSUInteger bytes = MIN(tail.frameLength * sizeof(float), SBMaxSeconds * SBRate * sizeof(float) - _samples.length);
            if (bytes) [_samples appendBytes:tail.floatChannelData[0] length:bytes];
            if (!bytes || status == AVAudioConverterOutputStatus_EndOfStream) break;
        }
    }
    return _samples;
}
@end

@interface SBPanel : NSPanel
- (void)showPhase:(NSString *)phase heard:(NSString *)heard detail:(NSString *)detail;
- (void)setMicLevel:(float)level;
- (void)updateMotion;
@end

static NSColor *SBTeal(void) { return [NSColor colorWithSRGBRed:.35 green:.85 blue:.75 alpha:1]; }
static NSTextField *SBLabel(CGFloat size, NSFontWeight weight) {
    NSTextField *label = [NSTextField wrappingLabelWithString:@""];
    label.font = [NSFont systemFontOfSize:size weight:weight];
    label.textColor = NSColor.whiteColor;
    return label;
}

@implementation SBPanel {
    NSImageView *_logo;
    NSTextField *_title, *_heard;
    NSButton *_dismiss, *_dashboard;
    NSLevelIndicator *_meter;
    NSScrollView *_scroll;
    NSTextView *_text;
    BOOL _active;
    NSScreen *_screen;
}
- (instancetype)init {
    self = [super initWithContentRect:NSMakeRect(0, 0, 344, 100)
        styleMask:NSWindowStyleMaskBorderless | NSWindowStyleMaskNonactivatingPanel
        backing:NSBackingStoreBuffered defer:NO];
    if (!self) return nil;
    self.level = NSFloatingWindowLevel;
    self.collectionBehavior = NSWindowCollectionBehaviorCanJoinAllSpaces |
        NSWindowCollectionBehaviorFullScreenAuxiliary | NSWindowCollectionBehaviorIgnoresCycle;
    self.becomesKeyOnlyIfNeeded = YES;
    self.hidesOnDeactivate = NO;
    self.releasedWhenClosed = NO;
    self.opaque = NO;
    self.backgroundColor = NSColor.clearColor;
    self.hasShadow = YES;
    self.title = @"StickyBot voice";
    NSView *v = self.contentView;
    v.wantsLayer = YES;
    v.layer.backgroundColor = [NSColor colorWithSRGBRed:.085 green:.115 blue:.12 alpha:.98].CGColor;
    v.layer.cornerRadius = 19;
    v.layer.borderWidth = 1;
    v.layer.borderColor = [NSColor colorWithWhite:.39 alpha:.65].CGColor;
    _logo = [NSImageView new];
    _logo.image = NSApp.applicationIconImage;
    _logo.imageScaling = NSImageScaleProportionallyUpOrDown;
    _logo.wantsLayer = YES;
    _logo.layer.shadowOffset = CGSizeZero;
    _logo.layer.shadowRadius = 6;
    _logo.layer.shadowColor = SBTeal().CGColor;
    _logo.accessibilityLabel = @"StickyBot";
    _title = SBLabel(13, NSFontWeightSemibold);
    _heard = SBLabel(12, NSFontWeightRegular);
    _heard.maximumNumberOfLines = 2;
    _heard.lineBreakMode = NSLineBreakByTruncatingTail;
    _heard.textColor = [NSColor colorWithWhite:.78 alpha:1];
    _dismiss = [NSButton buttonWithImage:[NSImage imageWithSystemSymbolName:@"xmark" accessibilityDescription:@"Dismiss"]
                                  target:self action:@selector(dismiss:)];
    _dismiss.bordered = NO;
    _dismiss.contentTintColor = NSColor.lightGrayColor;
    _dismiss.accessibilityLabel = @"Dismiss voice panel";
    _dashboard = [NSButton buttonWithTitle:@"Open dashboard" target:self action:@selector(dashboard:)];
    _dashboard.bezelStyle = NSBezelStyleInline;
    _dashboard.contentTintColor = NSColor.whiteColor;
    _meter = [NSLevelIndicator new];
    _meter.levelIndicatorStyle = NSLevelIndicatorStyleContinuousCapacity;
    _meter.maxValue = 1;
    _meter.fillColor = SBTeal();
    _meter.accessibilityLabel = @"Microphone level";
    _scroll = [NSScrollView new];
    _scroll.drawsBackground = NO;
    _scroll.autohidesScrollers = YES;
    _scroll.scrollerStyle = NSScrollerStyleOverlay;
    _text = [[NSTextView alloc] initWithFrame:NSMakeRect(0, 0, 312, 20)];
    _text.drawsBackground = NO;
    _text.editable = NO;
    _text.selectable = NO;
    _text.font = [NSFont systemFontOfSize:13];
    _text.textColor = [NSColor colorWithWhite:.9 alpha:1];
    _text.textContainerInset = NSZeroSize;
    _text.textContainer.lineFragmentPadding = 0;
    _text.textContainer.widthTracksTextView = YES;
    _text.verticallyResizable = YES;
    _text.autoresizingMask = NSViewWidthSizable;
    _scroll.documentView = _text;
    for (NSView *item in @[_logo, _title, _heard, _dismiss, _dashboard, _meter, _scroll]) [v addSubview:item];
    return self;
}
- (BOOL)canBecomeKeyWindow { return NO; }
- (BOOL)canBecomeMainWindow { return NO; }
- (void)dismiss:(id)sender { [self orderOut:nil]; }
- (void)dashboard:(id)sender { sb_voice_dashboard(); }
- (void)setMicLevel:(float)level { _meter.doubleValue = level; }
- (void)updateMotion {
    CALayer *layer = _logo.layer;
    [CATransaction begin];
    [CATransaction setDisableActions:YES];
    layer.shadowOpacity = _active ? .62 : 0;
    [CATransaction commit];
    if (!_active || NSWorkspace.sharedWorkspace.accessibilityDisplayShouldReduceMotion) {
        [layer removeAnimationForKey:@"voice-pulse"];
    } else if (![layer animationForKey:@"voice-pulse"]) {
        CABasicAnimation *scale = [CABasicAnimation animationWithKeyPath:@"transform.scale"];
        scale.fromValue = @1;
        scale.toValue = @1.07;
        CABasicAnimation *glow = [CABasicAnimation animationWithKeyPath:@"shadowOpacity"];
        glow.fromValue = @.36;
        glow.toValue = @.82;
        CAAnimationGroup *pulse = [CAAnimationGroup animation];
        pulse.animations = @[scale, glow];
        pulse.autoreverses = YES;
        pulse.duration = .9;
        pulse.repeatCount = HUGE_VALF;
        pulse.timingFunction = [CAMediaTimingFunction functionWithName:kCAMediaTimingFunctionEaseInEaseOut];
        [layer addAnimation:pulse forKey:@"voice-pulse"];
    }
}
- (void)showPhase:(NSString *)phase heard:(NSString *)heard detail:(NSString *)detail {
    NSDictionary *titles = @{@"listening":@"Listening", @"transcribing":@"Listening", @"thinking":@"Thinking",
        @"answering":@"Answering", @"approval":@"Waiting for approval", @"reply":@"Reply complete", @"error":@"Voice request"};
    BOOL recording = [phase isEqual:@"listening"];
    BOOL complete = [phase isEqual:@"reply"];
    _active = !complete && ![phase isEqual:@"error"];
    NSString *title = titles[phase] ?: @"StickyBot";
    BOOL announce = ![_title.stringValue isEqual:title] || !self.isVisible;
    _title.stringValue = title;
    if (recording) detail = @"Release to send · Esc to cancel";
    if ([phase isEqual:@"transcribing"]) detail = @"Microphone off · processing your words";
    _heard.stringValue = heard.length ? [NSString stringWithFormat:@"“%@”", heard] : @"";
    BOOL pickScreen = recording || !self.isVisible || !_screen;
    NSArray<NSScreen *> *screens = NSScreen.screens;
    if (pickScreen || ![screens containsObject:_screen]) {
        _screen = NSScreen.mainScreen;
        for (NSScreen *s in screens) if (NSPointInRect(NSEvent.mouseLocation, s.frame)) { _screen = s; break; }
    }
    NSRect visible = _screen.visibleFrame;
    CGFloat width = MIN(recording || [phase isEqual:@"transcribing"] ? 278 : 350, NSWidth(visible) - 12);
    CGFloat bodyWidth = width - 32;
    _scroll.frame = NSMakeRect(16, 12, bodyWidth, 20);
    _text.string = detail;
    _text.textContainer.containerSize = NSMakeSize(bodyWidth, CGFLOAT_MAX);
    [_text.layoutManager ensureLayoutForTextContainer:_text.textContainer];
    CGFloat fullHeight = ceil([_text.layoutManager usedRectForTextContainer:_text.textContainer].size.height);
    CGFloat textHeight = MIN(fullHeight, MIN(190, NSHeight(visible) / 3));
    BOOL longReply = complete && fullHeight > textHeight;
    CGFloat footer = longReply ? 28 : 0;
    CGFloat heardHeight = heard.length ? MIN(34, ceil([_heard.cell cellSizeForBounds:NSMakeRect(0,0,bodyWidth,60)].height)) + 4 : 0;
    CGFloat height = 66 + textHeight + heardHeight + footer;
    [self setFrame:NSMakeRect(round(NSMidX(visible) - width * .5), NSMinY(visible) + 14, width, height) display:YES];
    _logo.frame = NSMakeRect(11, height - 53, 46, 46);
    _title.frame = NSMakeRect(66, height - 39, width - 114, 19);
    _meter.hidden = !recording;
    _meter.frame = NSMakeRect(width - 90, height - 36, 71, 10);
    _dismiss.hidden = _active;
    _dismiss.frame = NSMakeRect(width - 36, height - 43, 26, 26);
    _heard.hidden = !heardHeight;
    _heard.frame = NSMakeRect(16, 12 + footer + textHeight, bodyWidth, heardHeight);
    _dashboard.hidden = !longReply;
    _dashboard.frame = NSMakeRect(13, 9, 105, 21);
    _scroll.hasVerticalScroller = fullHeight > textHeight;
    _scroll.frame = NSMakeRect(16, 12 + footer, bodyWidth, textHeight);
    [_text setFrameSize:NSMakeSize(bodyWidth, MAX(textHeight, fullHeight))];
    [self updateMotion];
    [self orderFrontRegardless];
    if (announce) NSAccessibilityPostNotificationWithUserInfo(self, NSAccessibilityAnnouncementRequestedNotification,
        @{NSAccessibilityAnnouncementKey:title, NSAccessibilityPriorityKey:@(NSAccessibilityPriorityMedium)});
}
@end

static const OSType SBHotkeySignature = 'SbVc';
static const UInt32 SBEscapeID = 1;

@interface SBVoice : NSObject
- (NSString *)configure:(BOOL)enable code:(UInt32)code modifiers:(UInt32)mods;
- (void)keyEvent:(UInt32)identifier pressed:(BOOL)pressed;
- (void)show:(NSString *)phase heard:(NSString *)heard detail:(NSString *)detail;
- (void)shutdown;
@property(nonatomic, copy) NSString *configError;
@end

static OSStatus SBHotkeyEvent(EventHandlerCallRef __unused next, EventRef event, void *user) {
    EventHotKeyID key;
    if (GetEventParameter(event, kEventParamDirectObject, typeEventHotKeyID, NULL, sizeof(key), NULL, &key) != noErr ||
        key.signature != SBHotkeySignature) return eventNotHandledErr;
    [(__bridge SBVoice *)user keyEvent:key.id pressed:GetEventKind(event) == kEventHotKeyPressed];
    return noErr;
}
static NSEventModifierFlags SBFlags(UInt32 mods) {
    return ((mods & shiftKey) ? NSEventModifierFlagShift : 0) |
           ((mods & controlKey) ? NSEventModifierFlagControl : 0) |
           ((mods & cmdKey) ? NSEventModifierFlagCommand : 0) |
           ((mods & optionKey) ? NSEventModifierFlagOption : 0);
}

@implementation SBVoice {
    EventHotKeyRef _shortcut, _escape;
    EventHandlerRef _handler;
    UInt32 _code, _mods, _identifier;
    BOOL _held, _pending, _requesting, _restorePanel;
    NSUInteger _suspended;
    SBPanel *_panel;
    SBRecording *_recording;
    NSTimer *_poll;
    NSTimeInterval _deadline;
}
- (instancetype)init {
    self = [super init];
    if (!self) return nil;
    _identifier = SBEscapeID;
    NSNotificationCenter *ws = NSWorkspace.sharedWorkspace.notificationCenter;
    for (NSString *name in @[NSWorkspaceWillSleepNotification, NSWorkspaceScreensDidSleepNotification, NSWorkspaceSessionDidResignActiveNotification])
        [ws addObserver:self selector:@selector(suspend:) name:name object:nil];
    for (NSString *name in @[NSWorkspaceDidWakeNotification, NSWorkspaceScreensDidWakeNotification, NSWorkspaceSessionDidBecomeActiveNotification])
        [ws addObserver:self selector:@selector(resume:) name:name object:nil];
    [ws addObserver:self selector:@selector(motionChanged:) name:NSWorkspaceAccessibilityDisplayOptionsDidChangeNotification object:nil];
    NSDistributedNotificationCenter *dc = NSDistributedNotificationCenter.defaultCenter;
    [dc addObserver:self selector:@selector(suspend:) name:@"com.apple.screenIsLocked" object:nil];
    [dc addObserver:self selector:@selector(resume:) name:@"com.apple.screenIsUnlocked" object:nil];
    return self;
}

- (NSString *)configure:(BOOL)enable code:(UInt32)code modifiers:(UInt32)mods {
    if (!enable) {
        if (_shortcut) { UnregisterEventHotKey(_shortcut); _shortcut = NULL; }
        [self finishRecording:NO error:nil];
        _held = NO;
        self.configError = nil;
        return nil;
    }
    NSString *failed = [self registerCode:code modifiers:mods];
    if (failed) return failed;
    AVAuthorizationStatus auth = [AVCaptureDevice authorizationStatusForMediaType:AVMediaTypeAudio];
    self.configError = (auth == AVAuthorizationStatusDenied || auth == AVAuthorizationStatusRestricted) ? SBMicDenied : nil;
    if (auth == AVAuthorizationStatusNotDetermined && !_requesting) {
        _requesting = YES;
        __weak SBVoice *weakSelf = self;
        [AVCaptureDevice requestAccessForMediaType:AVMediaTypeAudio completionHandler:^(BOOL granted) {
            dispatch_async(dispatch_get_main_queue(), ^{
                SBVoice *voice = weakSelf;
                if (!voice) return;
                voice->_requesting = NO;
                if (voice->_shortcut && !granted) {
                    voice.configError = SBMicDenied;
                    if (!voice->_pending) [voice show:@"error" heard:@"" detail:SBMicDenied];
                }
            });
        }];
    }
    return nil;
}

// Separated from permission prompting so binding conflicts can be exercised without opening the mic.
- (NSString *)registerCode:(UInt32)code modifiers:(UInt32)mods {
    if (!mods || (mods & ~(cmdKey | shiftKey | optionKey | controlKey)) || code > 127 || code == kVK_Escape)
        return @"Choose a key with Control, Option, Command or Shift.";
    if (!_handler) {
        EventTypeSpec events[] = {{kEventClassKeyboard,kEventHotKeyPressed}, {kEventClassKeyboard,kEventHotKeyReleased}};
        if (InstallEventHandler(GetApplicationEventTarget(), SBHotkeyEvent, 2, events, (__bridge void *)self, &_handler) != noErr)
            return @"macOS could not install the shortcut handler.";
    }
    if (!_shortcut || _code != code || _mods != mods) {
        // Acquire the replacement before releasing the working chord. Distinct IDs reject late key-ups.
        EventHotKeyID id = {SBHotkeySignature, _identifier + 1};
        EventHotKeyRef replacement = NULL;
        OSStatus status = RegisterEventHotKey(code, mods, id, GetApplicationEventTarget(), kEventHotKeyExclusive, &replacement);
        if (status != noErr) return @"This shortcut is unavailable. Choose another combination.";
        if (_shortcut) UnregisterEventHotKey(_shortcut);
        [self finishRecording:NO error:nil];
        _shortcut = replacement;
        _identifier = id.id;
        _code = code;
        _mods = mods;
        _held = NO;
    }
    return nil;
}

- (void)keyEvent:(UInt32)identifier pressed:(BOOL)pressed {
    if (identifier == SBEscapeID) {
        if (pressed && _recording) [self finishRecording:NO error:nil];
        return;
    }
    if (!_shortcut || identifier != _identifier) return;
    if (!pressed) {
        _held = NO;
        [self finishRecording:YES error:nil];
        return;
    }
    if (_held || _suspended) return;
    _held = YES;
    if (_pending || _recording) return;
    AVAuthorizationStatus permission = [AVCaptureDevice authorizationStatusForMediaType:AVMediaTypeAudio];
    if (permission != AVAuthorizationStatusAuthorized) {
        NSString *reason = permission == AVAuthorizationStatusNotDetermined ? @"Grant microphone access, then hold the shortcut again." : SBMicDenied;
        [self show:@"error" heard:@"" detail:reason];
        return;
    }
    self.configError = nil;
    if (!sb_voice_begin()) {
        [self show:@"error" heard:@"" detail:@"The agent is busy. Try again when this turn finishes."];
        return;
    }
    _recording = [SBRecording new];
    EventHotKeyID escapeID = {SBHotkeySignature, SBEscapeID};
    if (RegisterEventHotKey(kVK_Escape, _mods, escapeID, GetApplicationEventTarget(), kEventHotKeyExclusive, &_escape) != noErr) {
        [self finishRecording:NO error:@"Escape is already assigned with these modifiers. Choose another talk shortcut."];
        return;
    }
    NSString *error = [_recording start];
    if (error) { [self finishRecording:NO error:error]; return; }
    [NSNotificationCenter.defaultCenter addObserver:self selector:@selector(inputChanged:) name:AVAudioEngineConfigurationChangeNotification object:_recording.engine];
    [self show:@"listening" heard:@"" detail:@""];
    _deadline = NSProcessInfo.processInfo.systemUptime + SBMaxSeconds;
    _poll = [NSTimer timerWithTimeInterval:.05 target:self selector:@selector(tick:) userInfo:nil repeats:YES];
    [NSRunLoop.mainRunLoop addTimer:_poll forMode:NSRunLoopCommonModes];
}

- (void)tick:(NSTimer *)timer {
    if (!_recording) return;
    NSString *failure = _recording.failure;
    if (!failure && (_recording.device != SBInput() || !_recording.engine.isRunning))
        failure = @"The microphone disconnected or changed. Hold the shortcut to try again.";
    if (!failure && NSProcessInfo.processInfo.systemUptime >= _deadline)
        failure = @"Recording reached one minute. Hold the shortcut again for a shorter question.";
    if (failure) { [self finishRecording:NO error:failure]; return; }
    // Releasing a modifier first should stop capture even if the main key is still down.
    NSEventModifierFlags required = SBFlags(_mods);
    if ((NSEvent.modifierFlags & required) != required) {
        _held = NO;
        [self finishRecording:YES error:nil];
        return;
    }
    [_panel setMicLevel:_recording.level];
}

- (void)finishRecording:(BOOL)send error:(NSString *)reason {
    if (!_recording) return;
    [_poll invalidate]; _poll = nil;
    if (_escape) { UnregisterEventHotKey(_escape); _escape = NULL; }
    SBRecording *recording = _recording; _recording = nil;
    if (recording.engine) [NSNotificationCenter.defaultCenter removeObserver:self name:AVAudioEngineConfigurationChangeNotification object:recording.engine];
    if (send && (recording.device != SBInput() || !recording.engine.isRunning))
        reason = @"The microphone disconnected or changed. Hold the shortcut to try again.";
    NSData *samples = [recording finish];
    NSString *failure = reason ?: recording.failure;
    if (send && !failure && samples.length) {
        _pending = YES;
        [self show:@"transcribing" heard:@"" detail:@""];
        sb_voice_audio(samples.bytes, samples.length / sizeof(float));
    } else {
        sb_voice_cancel();
        if (send && !failure) failure = @"No audio captured. Hold the shortcut while you speak.";
        if (failure) [self show:@"error" heard:@"" detail:failure];
        else [_panel orderOut:nil];
    }
}

- (void)show:(NSString *)phase heard:(NSString *)heard detail:(NSString *)detail {
    if (!_panel) _panel = [SBPanel new];
    if ([phase isEqual:@"cancelled"]) {
        _restorePanel = NO;
        _pending = NO;
        [_panel orderOut:nil];
        return;
    }
    if ([phase isEqual:@"reply"] || [phase isEqual:@"error"]) _pending = NO;
    [_panel showPhase:phase heard:heard detail:detail];
    if (_suspended) { _restorePanel = YES; [_panel orderOut:nil]; }
}
- (void)inputChanged:(NSNotification *)note {
    // AVAudioEngine posts from its own dispatch queue. Check the original engine on main.
    dispatch_async(dispatch_get_main_queue(), ^{
        if (note.object == self->_recording.engine)
            [self finishRecording:NO error:@"The microphone changed. Hold the shortcut to try again."];
    });
}
// Waking a display does not unlock it. Match each notification with its own resume.
- (NSUInteger)suspensionReason:(NSString *)name {
    if ([name isEqual:NSWorkspaceWillSleepNotification] || [name isEqual:NSWorkspaceDidWakeNotification]) return 1;
    if ([name isEqual:NSWorkspaceScreensDidSleepNotification] || [name isEqual:NSWorkspaceScreensDidWakeNotification]) return 2;
    if ([name isEqual:NSWorkspaceSessionDidResignActiveNotification] || [name isEqual:NSWorkspaceSessionDidBecomeActiveNotification]) return 4;
    return 8;
}
- (void)suspend:(NSNotification *)note {
    if (!NSThread.isMainThread) { dispatch_async(dispatch_get_main_queue(), ^{ [self suspend:note]; }); return; }
    _suspended |= [self suspensionReason:note.name];
    _held = NO;
    [self finishRecording:NO error:nil];
    if (_panel.isVisible) _restorePanel = YES;
    [_panel orderOut:nil];
}
- (void)resume:(NSNotification *)note {
    if (!NSThread.isMainThread) { dispatch_async(dispatch_get_main_queue(), ^{ [self resume:note]; }); return; }
    _suspended &= ~[self suspensionReason:note.name];
    if (!_suspended && _restorePanel) { _restorePanel = NO; [_panel orderFrontRegardless]; }
}
- (void)motionChanged:(NSNotification *)note {
    dispatch_async(dispatch_get_main_queue(), ^{ [self->_panel updateMotion]; });
}
- (void)shutdown {
    [self configure:NO code:0 modifiers:0];
    if (_handler) { RemoveEventHandler(_handler); _handler = NULL; }
    [NSWorkspace.sharedWorkspace.notificationCenter removeObserver:self];
    [NSDistributedNotificationCenter.defaultCenter removeObserver:self];
    [_panel orderOut:nil];
}
@end

// All public entry points run on the AppKit main thread. UTF-8 C string results
// are borrowed; Rust copies them before returning to the main run loop.
static SBVoice *voice;
const char *sb_voice_configure(int enabled, uint32_t keycode, uint32_t carbon_mods) {
    if (!voice && enabled) voice = [SBVoice new];
    static NSString *configurationResult;
    configurationResult = [voice configure:enabled != 0 code:keycode modifiers:carbon_mods];
    return configurationResult.UTF8String;
}
const char *sb_voice_config_error(void) {
    return voice.configError.UTF8String;
}
void sb_voice_panel(const char *phase, const char *heard, const char *detail) {
    [voice show:[NSString stringWithUTF8String:phase] heard:[NSString stringWithUTF8String:heard] detail:[NSString stringWithUTF8String:detail]];
}
void sb_voice_shutdown(void) {
    [voice shutdown];
    voice = nil;
}
