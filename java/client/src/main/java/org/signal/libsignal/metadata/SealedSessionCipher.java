//
// Copyright 2023 Signal Messenger, LLC.
// SPDX-License-Identifier: AGPL-3.0-only
//

package org.signal.libsignal.metadata;

import static org.signal.libsignal.internal.FilterExceptions.filterExceptions;

import java.time.Instant;
import java.util.Collections;
import java.util.List;
import java.util.Optional;
import java.util.UUID;
import org.signal.libsignal.internal.Native;
import org.signal.libsignal.internal.NativeHandleGuard;
import org.signal.libsignal.metadata.certificate.CertificateValidator;
import org.signal.libsignal.metadata.certificate.SenderCertificate;
import org.signal.libsignal.metadata.protocol.UnidentifiedSenderMessageContent;
import org.signal.libsignal.protocol.DuplicateMessageException;
import org.signal.libsignal.protocol.InvalidKeyException;
import org.signal.libsignal.protocol.InvalidKeyIdException;
import org.signal.libsignal.protocol.InvalidMessageException;
import org.signal.libsignal.protocol.InvalidRegistrationIdException;
import org.signal.libsignal.protocol.InvalidVersionException;
import org.signal.libsignal.protocol.LegacyMessageException;
import org.signal.libsignal.protocol.NoSessionException;
import org.signal.libsignal.protocol.ServiceId;
import org.signal.libsignal.protocol.SessionCipher;
import org.signal.libsignal.protocol.SignalProtocolAddress;
import org.signal.libsignal.protocol.UntrustedIdentityException;
import org.signal.libsignal.protocol.UsePqRatchet;
import org.signal.libsignal.protocol.groups.GroupCipher;
import org.signal.libsignal.protocol.message.CiphertextMessage;
import org.signal.libsignal.protocol.message.PreKeySignalMessage;
import org.signal.libsignal.protocol.message.SignalMessage;
import org.signal.libsignal.protocol.state.SessionRecord;
import org.signal.libsignal.protocol.state.SignalProtocolStore;

public class SealedSessionCipher {

  private static final String TAG = SealedSessionCipher.class.getSimpleName();

  private final SignalProtocolStore signalProtocolStore;
  private final String localE164Address;
  private final String localUuidAddress;
  private final int localDeviceId;

  public SealedSessionCipher(
      SignalProtocolStore signalProtocolStore,
      UUID localUuid,
      String localE164Address,
      int localDeviceId) {
    this.signalProtocolStore = signalProtocolStore;
    this.localUuidAddress = localUuid.toString();
    this.localE164Address = localE164Address;
    this.localDeviceId = localDeviceId;
  }

  public byte[] encrypt(
      SignalProtocolAddress destinationAddress,
      SenderCertificate senderCertificate,
      byte[] paddedPlaintext)
      throws InvalidKeyException, UntrustedIdentityException {
    try (NativeHandleGuard addressGuard = new NativeHandleGuard(destinationAddress)) {
      CiphertextMessage message =
          filterExceptions(
              InvalidKeyException.class,
              UntrustedIdentityException.class,
              () ->
                  Native.SessionCipher_EncryptMessage(
                      paddedPlaintext,
                      addressGuard.nativeHandle(),
                      this.signalProtocolStore,
                      this.signalProtocolStore,
                      Instant.now().toEpochMilli()));
      UnidentifiedSenderMessageContent content =
          new UnidentifiedSenderMessageContent(
              message,
              senderCertificate,
              UnidentifiedSenderMessageContent.CONTENT_HINT_DEFAULT,
              Optional.<byte[]>empty());
      return encrypt(destinationAddress, content);
    }
  }

  public byte[] encrypt(
      SignalProtocolAddress destinationAddress, UnidentifiedSenderMessageContent content)
      throws InvalidKeyException, UntrustedIdentityException {
    try (NativeHandleGuard addressGuard = new NativeHandleGuard(destinationAddress);
        NativeHandleGuard contentGuard = new NativeHandleGuard(content)) {
      return filterExceptions(
          InvalidKeyException.class,
          UntrustedIdentityException.class,
          () ->
              Native.SealedSessionCipher_Encrypt(
                  addressGuard.nativeHandle(),
                  contentGuard.nativeHandle(),
                  this.signalProtocolStore));
    }
  }

  public byte[] multiRecipientEncrypt(
      List<SignalProtocolAddress> recipients, UnidentifiedSenderMessageContent content)
      throws InvalidKeyException,
          InvalidRegistrationIdException,
          NoSessionException,
          UntrustedIdentityException {
    return multiRecipientEncrypt(recipients, content, Collections.emptyList());
  }

  public byte[] multiRecipientEncrypt(
      List<SignalProtocolAddress> recipients,
      UnidentifiedSenderMessageContent content,
      List<ServiceId> excludedRecipients)
      throws InvalidKeyException,
          InvalidRegistrationIdException,
          NoSessionException,
          UntrustedIdentityException {
    List<SessionRecord> recipientSessions =
        this.signalProtocolStore.loadExistingSessions(recipients);
    return multiRecipientEncrypt(recipients, recipientSessions, content, excludedRecipients);
  }

  public byte[] multiRecipientEncrypt(
      List<SignalProtocolAddress> recipients,
      List<SessionRecord> recipientSessions,
      UnidentifiedSenderMessageContent content)
      throws InvalidKeyException,
          InvalidRegistrationIdException,
          NoSessionException,
          UntrustedIdentityException {
    return multiRecipientEncrypt(recipients, recipientSessions, content, Collections.emptyList());
  }

  public byte[] multiRecipientEncrypt(
      List<SignalProtocolAddress> recipients,
      List<SessionRecord> recipientSessions,
      UnidentifiedSenderMessageContent content,
      List<ServiceId> excludedRecipients)
      throws InvalidKeyException,
          InvalidRegistrationIdException,
          NoSessionException,
          UntrustedIdentityException {
    if (recipients.size() != recipientSessions.size()) {
      throw new IllegalArgumentException("Size of recipients and sessions do not match");
    }
    // Unsafely access the native handles for the recipients and sessions,
    // because try-with-resources syntax doesn't support a List of resources.
    long[] recipientHandles = new long[recipients.size()];
    int i = 0;
    for (SignalProtocolAddress nextRecipient : recipients) {
      recipientHandles[i] = nextRecipient.unsafeNativeHandleWithoutGuard();
      i++;
    }

    long[] recipientSessionHandles = new long[recipientSessions.size()];
    i = 0;
    for (SessionRecord nextSession : recipientSessions) {
      recipientSessionHandles[i] = nextSession.unsafeNativeHandleWithoutGuard();
      i++;
    }

    try (NativeHandleGuard contentGuard = new NativeHandleGuard(content)) {
      byte[] result =
          filterExceptions(
              InvalidKeyException.class,
              InvalidRegistrationIdException.class,
              NoSessionException.class,
              UntrustedIdentityException.class,
              () ->
                  Native.SealedSessionCipher_MultiRecipientEncrypt(
                      recipientHandles,
                      recipientSessionHandles,
                      ServiceId.toConcatenatedFixedWidthBinary(excludedRecipients),
                      contentGuard.nativeHandle(),
                      this.signalProtocolStore));
      // Manually keep the lists of recipients and sessions from being garbage collected
      // while we're using their native handles.
      Native.keepAlive(recipients);
      Native.keepAlive(recipientSessions);
      return result;
    }
  }

  // For testing only.
  static byte[] multiRecipientMessageForSingleRecipient(byte[] message) {
    return filterExceptions(
        () -> Native.SealedSessionCipher_MultiRecipientMessageForSingleRecipient(message));
  }

  public DecryptionResult decrypt(
      CertificateValidator validator, byte[] ciphertext, long timestamp, UsePqRatchet usePqRatchet)
      throws InvalidMetadataMessageException,
          InvalidMetadataVersionException,
          ProtocolInvalidMessageException,
          ProtocolInvalidKeyException,
          ProtocolNoSessionException,
          ProtocolLegacyMessageException,
          ProtocolInvalidVersionException,
          ProtocolDuplicateMessageException,
          ProtocolInvalidKeyIdException,
          ProtocolUntrustedIdentityException,
          SelfSendException {
    UnidentifiedSenderMessageContent content;
    try {
      content =
          new UnidentifiedSenderMessageContent(
              Native.SealedSessionCipher_DecryptToUsmc(ciphertext, this.signalProtocolStore));
      validator.validate(content.getSenderCertificate(), timestamp);
    } catch (Exception e) {
      throw new InvalidMetadataMessageException(e);
    }

    boolean isLocalE164 =
        localE164Address != null
            && localE164Address.equals(content.getSenderCertificate().getSenderE164().orElse(null));
    boolean isLocalUuid = localUuidAddress.equals(content.getSenderCertificate().getSenderUuid());

    if ((isLocalE164 || isLocalUuid)
        && content.getSenderCertificate().getSenderDeviceId() == localDeviceId) {
      throw new SelfSendException();
    }

    try {
      return new DecryptionResult(
          content.getSenderCertificate().getSenderUuid(),
          content.getSenderCertificate().getSenderE164(),
          content.getSenderCertificate().getSenderDeviceId(),
          content.getType(),
          content.getGroupId(),
          decrypt(content, usePqRatchet));
    } catch (InvalidMessageException e) {
      throw new ProtocolInvalidMessageException(e, content);
    } catch (InvalidKeyException e) {
      throw new ProtocolInvalidKeyException(e, content);
    } catch (NoSessionException e) {
      throw new ProtocolNoSessionException(e, content);
    } catch (LegacyMessageException e) {
      throw new ProtocolLegacyMessageException(e, content);
    } catch (InvalidVersionException e) {
      throw new ProtocolInvalidVersionException(e, content);
    } catch (DuplicateMessageException e) {
      throw new ProtocolDuplicateMessageException(e, content);
    } catch (InvalidKeyIdException e) {
      throw new ProtocolInvalidKeyIdException(e, content);
    } catch (UntrustedIdentityException e) {
      throw new ProtocolUntrustedIdentityException(e, content);
    }
  }

  public int getSessionVersion(SignalProtocolAddress remoteAddress) {
    return new SessionCipher(signalProtocolStore, remoteAddress).getSessionVersion();
  }

  public int getRemoteRegistrationId(SignalProtocolAddress remoteAddress) {
    return new SessionCipher(signalProtocolStore, remoteAddress).getRemoteRegistrationId();
  }

  private byte[] decrypt(UnidentifiedSenderMessageContent message, UsePqRatchet usePqRatchet)
      throws InvalidVersionException,
          InvalidMessageException,
          InvalidKeyException,
          DuplicateMessageException,
          InvalidKeyIdException,
          UntrustedIdentityException,
          LegacyMessageException,
          NoSessionException {
    SignalProtocolAddress sender =
        new SignalProtocolAddress(
            message.getSenderCertificate().getSenderUuid(),
            message.getSenderCertificate().getSenderDeviceId());

    switch (message.getType()) {
      case CiphertextMessage.WHISPER_TYPE:
        return new SessionCipher(signalProtocolStore, sender)
            .decrypt(new SignalMessage(message.getContent()));
      case CiphertextMessage.PREKEY_TYPE:
        return new SessionCipher(signalProtocolStore, sender)
            .decrypt(new PreKeySignalMessage(message.getContent()), usePqRatchet);
      case CiphertextMessage.SENDERKEY_TYPE:
        return new GroupCipher(signalProtocolStore, sender).decrypt(message.getContent());
      case CiphertextMessage.PLAINTEXT_CONTENT_TYPE:
        return filterExceptions(
            InvalidMessageException.class,
            InvalidVersionException.class,
            () -> Native.PlaintextContent_DeserializeAndGetContent(message.getContent()));
      default:
        throw new InvalidMessageException("Unknown type: " + message.getType());
    }
  }

  public static class DecryptionResult {
    private final String senderUuid;
    private final Optional<String> senderE164;
    private final int deviceId;
    private final int messageType;
    private final Optional<byte[]> groupId;
    private final byte[] paddedMessage;

    private DecryptionResult(
        String senderUuid,
        Optional<String> senderE164,
        int deviceId,
        int messageType,
        Optional<byte[]> groupId,
        byte[] paddedMessage) {
      this.senderUuid = senderUuid;
      this.senderE164 = senderE164;
      this.deviceId = deviceId;
      this.messageType = messageType;
      this.groupId = groupId;
      this.paddedMessage = paddedMessage;
    }

    public String getSenderUuid() {
      return senderUuid;
    }

    /**
     * Returns an ACI if the sender is a valid UUID, {@code null} otherwise.
     *
     * <p>In a future release DecryptionResult will <em>only</em> support ACIs.
     */
    public ServiceId.Aci getSenderAci() {
      try {
        return ServiceId.Aci.parseFromString(getSenderUuid());
      } catch (ServiceId.InvalidServiceIdException e) {
        return null;
      }
    }

    public Optional<String> getSenderE164() {
      return senderE164;
    }

    public int getDeviceId() {
      return deviceId;
    }

    public int getCiphertextMessageType() {
      return messageType;
    }

    public byte[] getPaddedMessage() {
      return paddedMessage;
    }

    public Optional<byte[]> getGroupId() {
      return groupId;
    }
  }
}
